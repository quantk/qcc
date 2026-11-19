//! The system's notifications, which plugins ask for through the daemon
//! (`notification.show`). The webview decides whether one is worth showing; this
//! shows it, and on desktops that say when a notification is clicked, brings the
//! window up and tells the webview which thread to open.

use tauri::{AppHandle, Emitter, Manager};

/// Told to the webview with the thread of a notification the user clicked.
pub const CLICKED_EVENT: &str = "notification-clicked";

/// Bring the window to the front and open `thread` in it.
fn open_thread(app: &AppHandle, thread: String) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit(CLICKED_EVENT, thread);
}

/// Show a notification. Blocks while the system takes it.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn show(app: AppHandle, title: &str, body: &str, thread: Option<String>) -> Result<(), String> {
    xdg::show(app, title, body, thread)
}

/// Show a notification. Blocks while the system takes it. Clicks are not reported
/// here, so it opens nothing.
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn show(app: AppHandle, title: &str, body: &str, thread: Option<String>) -> Result<(), String> {
    let _ = (app, thread, open_thread);
    notify_rust::Notification::new()
        .appname("qcc")
        .summary(title)
        .body(body)
        .show()
        .map(drop)
        .map_err(|error| error.to_string())
}

/// Freedesktop notifications, over D-Bus.
#[cfg(all(unix, not(target_os = "macos")))]
mod xdg {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::AppHandle;

    use super::open_thread;

    /// What the notification server sends when the notification itself is clicked.
    const CLICK: &str = "default";
    /// Notifications whose click the app waits for, each on a thread of its own
    /// until the notification goes away. Some desktops keep them for long; past
    /// this many, more are shown without opening anything on a click.
    const MAX_WAITING: usize = 16;
    static WAITING: AtomicUsize = AtomicUsize::new(0);

    /// Take a place among the notifications waited on, if one is free.
    fn reserve_waiter() -> bool {
        let mut waiting = WAITING.load(Ordering::SeqCst);
        while waiting < MAX_WAITING {
            match WAITING.compare_exchange(waiting, waiting + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return true,
                Err(now) => waiting = now,
            }
        }
        false
    }

    /// The body is markup to most servers: what the agent wrote must stay text.
    pub(super) fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub(super) fn show(
        app: AppHandle,
        title: &str,
        body: &str,
        thread: Option<String>,
    ) -> Result<(), String> {
        let mut notification = notify_rust::Notification::new();
        notification.appname("qcc").summary(title).body(&escape(body));
        let clickable = thread.filter(|_| reserve_waiter());
        if clickable.is_some() {
            notification.action(CLICK, "Open");
        }
        let shown = notification.show();
        let Some(thread) = clickable else {
            return shown.map(drop).map_err(|error| error.to_string());
        };
        let handle = match shown {
            Ok(handle) => handle,
            Err(error) => {
                WAITING.fetch_sub(1, Ordering::SeqCst);
                return Err(error.to_string());
            }
        };
        std::thread::spawn(move || {
            handle.wait_for_action(|action| {
                if action == CLICK {
                    open_thread(&app, thread);
                }
            });
            WAITING.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn what_the_agent_wrote_is_not_taken_for_markup() {
            assert_eq!(super::escape("<b>a & b</b>"), "&lt;b&gt;a &amp; b&lt;/b&gt;");
        }
    }
}
