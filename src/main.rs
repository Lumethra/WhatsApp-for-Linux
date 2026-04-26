use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Button, Box as GtkBox, Orientation,
    CssProvider, StyleContext, Overlay, Clipboard,
};
use glib::clone;
use gtk::gdk;
use gtk::gdk::Screen;
use gio::prelude::OutputStreamExt;
use gio::traits::MemoryOutputStreamExt;

use webkit2gtk::traits::*;
use webkit2gtk::{
    WebView, WebContext, UserContentManager, UserScript, UserContentInjectedFrames,
    UserScriptInjectionTime, JavascriptResult, Settings, HardwareAccelerationPolicy,
    ProcessModel, CacheModel,
};

use notify_rust::Notification;
use serde_json::Value;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

fn main() {
    let app = Application::builder()
        .application_id("com.example.whatsapp")
        .build();

    app.connect_activate(|app| {
        // ---------------- CSS LOADING (GTK3) ----------------
        let provider = CssProvider::new();
        provider
            .load_from_data(include_bytes!("style.css"))
            .expect("Failed to load CSS");

        StyleContext::add_provider_for_screen(
            &Screen::default().expect("No screen"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // ---------------- WEBVIEW SETUP ----------------
        let ctx = WebContext::default().unwrap();
        ctx.set_cache_model(CacheModel::WebBrowser);
        ctx.set_process_model(ProcessModel::SharedSecondaryProcess);

        let manager = UserContentManager::new();
        manager.register_script_message_handler("external");
        manager.register_script_message_handler("badge");
        manager.register_script_message_handler("theme");

        let settings = Settings::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
            .javascript_can_access_clipboard(true)
            .enable_smooth_scrolling(true)
            .enable_dns_prefetching(true)
            .enable_page_cache(true)
            .enable_offline_web_application_cache(true)
            .enable_html5_database(true)
            .enable_html5_local_storage(true)
            .hardware_acceleration_policy(HardwareAccelerationPolicy::Always)
            .build();

        let webview = WebView::builder()
            .web_context(&ctx)
            .user_content_manager(&manager)
            .settings(&settings)
            .build();

        // ---------------- JS BRIDGE + DARK MODE DETECTION ----------------
        let js_bridge = r#"
            (function() {
                const notify = (title, options) => {
                    if (window.lastNotification === title + options?.body) return;
                    window.lastNotification = title + options?.body;
                    setTimeout(() => { window.lastNotification = null; }, 2000);
                    window.webkit.messageHandlers.external.postMessage(JSON.stringify({ title, body: options?.body || "" }));
                };

                window.Notification = function(title, options) {
                    notify(title, options);
                    return { close: () => {}, onclick: null, addEventListener: () => {} };
                };
                window.Notification.permission = 'granted';

                if (window.ServiceWorkerRegistration) {
                    window.ServiceWorkerRegistration.prototype.showNotification = function(title, options) {
                        notify(title, options);
                        return Promise.resolve();
                    };
                }

                const updateBadge = () => {
                    const match = document.title.match(/\((\d+)\)/);
                    const count = match ? match[1] : "0";
                    window.webkit.messageHandlers.badge.postMessage(count);
                };

                const observer = new MutationObserver(updateBadge);
                observer.observe(document.querySelector('title'), { childList: true });
                updateBadge();

                if (navigator.permissions) {
                    const oldQuery = navigator.permissions.query;
                    navigator.permissions.query = function(spec) {
                        return spec.name === 'notifications'
                            ? Promise.resolve({ state: 'granted', onchange: null })
                            : oldQuery.apply(this, arguments);
                    };
                }

                // ---- Detect WhatsApp dark mode ----
                let lastTheme = { value: null };
                const sendTheme = () => {
                    if (!document.body) return;
                    const isDark = document.body.classList.contains("dark");
                    const theme = isDark ? "dark" : "light";
                    if (theme === lastTheme.value) return;
                    if (theme === "light" && lastTheme.value === null) return;
                    lastTheme.value = theme;
                    window.webkit.messageHandlers.theme.postMessage(theme);
                };

                // Only check theme when the page is idle — not during loading/navigation
                let debounceTimer = null;
                const sendThemeWhenIdle = () => {
                    clearTimeout(debounceTimer);
                    debounceTimer = setTimeout(sendTheme, 500);
                };

                const startThemeWatch = () => {
                    if (!document.body) {
                        setTimeout(startThemeWatch, 200);
                        return;
                    }
                    // Wait for initial load to fully settle
                    setTimeout(sendTheme, 2000);
                    new MutationObserver(sendThemeWhenIdle).observe(document.body, {
                        attributes: true,
                        attributeFilter: ["class"]
                    });
                };
                startThemeWatch();

            })();
        "#;

        manager.add_script(&UserScript::new(
            js_bridge,
            UserContentInjectedFrames::TopFrame,
            UserScriptInjectionTime::End,
            &[],
            &[],
        ));

        // ---------------- WINDOW SETUP ----------------
        let window = ApplicationWindow::builder()
            .application(app)
            .default_width(1100)
            .default_height(800)
            .decorated(false)
            .build();

        // ---------------- CUSTOM TITLE BAR ----------------
        let titlebar = GtkBox::new(Orientation::Horizontal, 8);
        titlebar.set_widget_name("custom-titlebar");
        titlebar.set_size_request(-1, 32);
        titlebar.style_context().add_class("dark-titlebar");

        let title_label = gtk::Label::new(Some("WhatsApp"));
        titlebar.pack_start(&title_label, false, false, 0);

        titlebar.set_hexpand(true);
        titlebar.set_vexpand(false);

        titlebar.connect_button_press_event(
            clone!(@weak window => @default-return Inhibit(false), move |_, event| {
                if event.button() == 1 {
                    let (root_x, root_y) = event.root();
                    window.begin_move_drag(
                        event.button() as i32,
                        root_x as i32,
                        root_y as i32,
                        event.time(),
                    );
                }
                Inhibit(false)
            }),
        );

        // ---------------- WINDOW BUTTONS + HIDE BUTTON ----------------
        let hide_btn = Button::with_label("▴"); // hide title bar

        let minimize = Button::with_label("🗕");
        let maximize = Button::with_label("🗗︎");
        let close = Button::with_label("🗙︎");

        minimize.style_context().add_class("title-minimize");
        maximize.style_context().add_class("title-maximize");
        close.style_context().add_class("title-close");

        minimize.connect_clicked(clone!(@weak window => move |_| window.iconify()));
        maximize.connect_clicked(clone!(@weak window => move |_| {
            if window.is_maximized() { window.unmaximize(); }
            else { window.maximize(); }
        }));
        close.connect_clicked(clone!(@weak window => move |_| window.close()));

        let right_box = GtkBox::new(Orientation::Horizontal, 4);
        right_box.pack_start(&hide_btn, false, false, 0);
        right_box.pack_start(&minimize, false, false, 0);
        right_box.pack_start(&maximize, false, false, 0);
        right_box.pack_start(&close, false, false, 0);

        titlebar.pack_end(&right_box, false, false, 0);

        // ---------------- OVERLAY SHOW BUTTON ----------------
        let overlay = Overlay::new();
        overlay.add(&webview);

        let show_btn = Button::with_label("▼");
        show_btn.set_halign(gtk::Align::Center);
        show_btn.set_valign(gtk::Align::Start);
        show_btn.style_context().add_class("show-titlebar");

        show_btn.set_no_show_all(true); 
        show_btn.hide();

        overlay.add_overlay(&show_btn);

        // ---------------- LAYOUT ----------------
        let layout = GtkBox::new(Orientation::Vertical, 0);
        layout.pack_start(&titlebar, false, false, 0);
        layout.pack_start(&overlay, true, true, 0);

        window.add(&layout);

        // ---------------- HIDE / SHOW LOGIC ----------------
        let titlebar_for_hide = titlebar.clone();
        let show_btn_for_hide = show_btn.clone();
        let show_btn_ctx = show_btn.style_context();
        hide_btn.connect_clicked(move |_| {
            titlebar_for_hide.hide();
            show_btn_for_hide.show();
            show_btn_ctx.add_class("opacity");

            glib::timeout_add_local(
                std::time::Duration::from_secs(1),
                clone!(@weak show_btn_ctx => @default-return glib::Continue(false), move || {
                    show_btn_ctx.remove_class("opacity");
                    glib::Continue(false)
                }),
            );
        });

        let titlebar_for_show = titlebar.clone();
        let show_btn_for_show = show_btn.clone();
        show_btn.connect_clicked(move |_| {
            show_btn_for_show.hide();
            titlebar_for_show.show();
        });

        // ---------------- NOTIFICATION HANDLER ----------------
        manager.connect_script_message_received(Some("external"), |_, result: &JavascriptResult| {
            if let Some(js_value) = result.js_value() {
                if let Ok(data) = serde_json::from_str::<Value>(&js_value.to_string()) {
                    let _ = Notification::new()
                        .summary(data["title"].as_str().unwrap_or("WhatsApp"))
                        .body(data["body"].as_str().unwrap_or(""))
                        .icon("whatsapp")
                        .show();
                }
            }
        });

        // ---------------- BADGE COUNTER ----------------
        let window_clone = window.clone();
        manager.connect_script_message_received(Some("badge"), move |_, result| {
            if let Some(js_value) = result.js_value() {
                let count = js_value.to_string();
                if count != "0" {
                    window_clone.set_title(&format!("WhatsApp ({})", count));
                } else {
                    window_clone.set_title("WhatsApp");
                }
            }
        });

        // ---------------- THEME HANDLER (DARK/LIGHT) ----------------
        let titlebar_clone = titlebar.clone();
        let light_pending = std::rc::Rc::new(std::cell::Cell::new(0u32));
        manager.connect_script_message_received(Some("theme"), move |_, result| {
            if let Some(js_value) = result.js_value() {
                let raw = js_value.to_string();
                let theme = raw.trim_matches('"');
                let ctx = titlebar_clone.style_context();
                let show_btn_ctx = show_btn.style_context();

                if theme == "dark" {
                    light_pending.set(light_pending.get().wrapping_add(1));
                    ctx.add_class("dark-titlebar");
                    ctx.remove_class("light-titlebar");
                    show_btn_ctx.add_class("dark-titlebar");
                    show_btn_ctx.remove_class("light-titlebar");
                } else {
                    let token = light_pending.get().wrapping_add(1);
                    light_pending.set(token);
                    let lp = light_pending.clone();
                    let ctx = ctx.clone();
                    let show_btn_ctx = show_btn_ctx.clone();
                    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
                        if lp.get() == token {
                            ctx.add_class("light-titlebar");
                            ctx.remove_class("dark-titlebar");
                            show_btn_ctx.add_class("light-titlebar");
                            show_btn_ctx.remove_class("dark-titlebar");
                        }
                        glib::Continue(false)
                    });
                }
            }
        });

        // ---------------- KEYBOARD SHORTCUTS ----------------
        let wv_clone = webview.clone();
        let wv_paste = webview.clone();
        window.connect_key_press_event(move |_, key_event| {
            let key = key_event.keyval();
            let ctrl = key_event.state().contains(gdk::ModifierType::CONTROL_MASK);

            if key == gdk::keys::constants::r && ctrl {
                wv_clone.reload();
                Inhibit(true)
            } else if key == gdk::keys::constants::v && ctrl {
                let clipboard = Clipboard::get(&gdk::Atom::intern("CLIPBOARD"));
                let wv = wv_paste.clone();
                clipboard.request_image(move |_, pixbuf| {
                    let Some(pixbuf) = pixbuf else { return };
                    let stream = gio::MemoryOutputStream::new_resizable();
                    if pixbuf.save_to_streamv(&stream, "png", &[], gio::Cancellable::NONE).is_ok() {
                        stream.close(gio::Cancellable::NONE).ok();
                        let bytes = stream.steal_as_bytes();
                        let b64 = BASE64.encode(bytes.as_ref());
                        let js = format!(
                            r#"(async () => {{
                                const r = await fetch('data:image/png;base64,{}');
                                const blob = await r.blob();
                                const file = new File([blob], 'paste.png', {{ type: 'image/png' }});
                                const dt = new DataTransfer();
                                dt.items.add(file);
                                const input = document.querySelector('[contenteditable="true"][data-tab]');
                                if (input) {{
                                    input.focus();
                                    input.dispatchEvent(new ClipboardEvent('paste', {{
                                        bubbles: true, cancelable: true, clipboardData: dt
                                    }}));
                                }}
                            }})()"#,
                            b64
                        );
                        wv.run_javascript(&js, gio::Cancellable::NONE, |_| {});
                    }
                });
                // Don't inhibit — let text paste still work if no image
                Inhibit(false)
            } else {
                Inhibit(false)
            }
        });

        // ---------------- LOAD WHATSAPP ----------------
        webview.load_uri("https://web.whatsapp.com");

        window.show_all();
    });

    app.run();
}
