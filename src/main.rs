use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Button, Box as GtkBox, Orientation,
    CssProvider, StyleContext,
};
use glib::clone;
use gtk::gdk;
use gtk::gdk::Screen;

use webkit2gtk::traits::*;
use webkit2gtk::{
    WebView, UserContentManager, UserScript, UserContentInjectedFrames, 
    UserScriptInjectionTime, JavascriptResult, Settings
};

use notify_rust::Notification;
use serde_json::Value;

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
        let manager = UserContentManager::new();
        manager.register_script_message_handler("external");
        manager.register_script_message_handler("badge"); 

        let settings = Settings::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
            .build();

        let webview = WebView::builder()
            .user_content_manager(&manager)
            .settings(&settings)
            .build();

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
                        return spec.name === 'notifications' ? Promise.resolve({ state: 'granted', onchange: null }) : oldQuery.apply(this, arguments);
                    };
                }
            })();
        "#;

        manager.add_script(&UserScript::new(
            js_bridge,
            UserContentInjectedFrames::AllFrames,
            UserScriptInjectionTime::End, 
            &[],
            &[],
        ));

        // ---------------- WINDOW SETUP ----------------
        let window = ApplicationWindow::builder()
            .application(app)
            .default_width(1100)
            .default_height(800)
            .decorated(false) // REMOVE SYSTEM TITLE BAR
            .build();

        // ---------------- CUSTOM TITLE BAR (GTK3) ----------------
        let titlebar = GtkBox::new(Orientation::Horizontal, 8);
        titlebar.set_widget_name("custom-titlebar");
        titlebar.set_size_request(-1, 32);

        // Force the bar to be visible
        let title_label = gtk::Label::new(Some("WhatsApp"));
        titlebar.pack_start(&title_label, false, false, 0);

        // Make sure GTK3 doesn't collapse it
        titlebar.set_hexpand(true);
        titlebar.set_vexpand(false);

        // Drag window
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

        // Buttons (GTK3 uses with_label)
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
        right_box.pack_start(&minimize, false, false, 0);
        right_box.pack_start(&maximize, false, false, 0);
        right_box.pack_start(&close, false, false, 0);

        titlebar.pack_end(&right_box, false, false, 0);

        // ---------------- LAYOUT (TITLEBAR + WEBVIEW) ----------------
        let layout = GtkBox::new(Orientation::Vertical, 0);
        layout.pack_start(&titlebar, false, false, 0);
        layout.pack_start(&webview, true, true, 0);

        window.add(&layout);

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

        // ---------------- KEYBOARD SHORTCUTS ----------------
        let wv_clone = webview.clone();
        window.connect_key_press_event(move |_, key_event| {
            let key = key_event.keyval();
            let ctrl = key_event.state().contains(gdk::ModifierType::CONTROL_MASK);

            if key == gdk::keys::constants::r && ctrl {
                wv_clone.reload();
                Inhibit(true)
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
