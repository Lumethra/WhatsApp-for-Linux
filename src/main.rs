use gtk::prelude::*;
use gtk::{Application, ApplicationWindow};
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
                // --- Notifications ---
                const notify = (title, options) => {
                    if (window.lastNotification === title + options?.body) return;
                    window.lastNotification = title + options?.body;
                    setTimeout(() => { window.lastNotification = null; }, 2000);
                    window.webkit.messageHandlers.external.postMessage(JSON.stringify({ title, body: options?.body || "" }));
                };
                window.Notification = function(title, options) { notify(title, options); return { close: () => {}, onclick: null, addEventListener: () => {} }; };
                window.Notification.permission = 'granted';
                if (window.ServiceWorkerRegistration) {
                    window.ServiceWorkerRegistration.prototype.showNotification = function(title, options) { notify(title, options); return Promise.resolve(); };
                }

                // --- Badge Counter (Title Observer) ---
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

        webview.load_uri("https://web.whatsapp.com");

        let window = ApplicationWindow::builder()
            .application(app)
            .title("WhatsApp")
            .default_width(1100)
            .default_height(800)
            .child(&webview)
            .build();

        // Handler 2: Badge Counter
        let window_clone = window.clone();
        manager.connect_script_message_received(Some("badge"), move |_, result: &JavascriptResult| {
            if let Some(js_value) = result.js_value() {
                let count = js_value.to_string();
                if count != "0" {
                    window_clone.set_title(&format!("WhatsApp ({})", count));
                } else {
                    window_clone.set_title("WhatsApp");
                }
            }
        });

        // -------- KEYBOARD SHORTCUTS ---------
        let wv_clone = webview.clone();
        window.connect_key_press_event(move |_, key_event| {
            let key = key_event.keyval();
            let state = key_event.state();
            let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);

            if key == gtk::gdk::keys::constants::r && ctrl {
                wv_clone.reload();
                gtk::Inhibit(true) 
            } else {
                gtk::Inhibit(false) 
            }
        });

        window.show_all();
    });

    app.run();
}
