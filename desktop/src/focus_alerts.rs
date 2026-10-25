//! A focus phase that ends while another app is in front says so through the
//! system's notifications; in front, chrome's own notice is enough.

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::atomic::{AtomicBool, Ordering};

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationSound, UNUserNotificationCenter,
    };

    static ASKED: AtomicBool = AtomicBool::new(false);

    /// The notification center exists only for a bundled app; an unbundled
    /// development binary raises on first touch instead of returning nothing.
    fn bundled() -> bool {
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
    }

    /// Asked once, when the first session starts, so the prompt arrives with
    /// the action it is for rather than at launch.
    pub fn ask() {
        if !bundled() || ASKED.swap(true, Ordering::AcqRel) {
            return;
        }
        let done = RcBlock::new(|_granted: Bool, _error: *mut NSError| {});
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &done,
            );
    }

    /// One identifier per subject, so a newer alert replaces the last
    /// instead of stacking a column of stale ones.
    pub fn post(identifier: &str, title: &str, body: &str) {
        if !bundled() {
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(identifier),
            &content,
            None,
        );
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, None);
    }
}

/// The session started: the moment to ask for permission to notify.
pub fn session_started() {
    ask_to_notify();
}

/// Asks once for permission to notify, when the first alert is near.
pub(crate) fn ask_to_notify() {
    #[cfg(target_os = "macos")]
    macos::ask();
}

/// Posts a system notification; a later one with the same identifier replaces it.
pub(crate) fn notify(identifier: &str, title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    macos::post(identifier, title, body);
    #[cfg(not(target_os = "macos"))]
    let _ = (identifier, title, body);
}

/// One of Zephium's windows is in front, so chrome's own notice is enough.
pub(crate) fn in_front(app: &tauri::AppHandle) -> bool {
    use tauri::Manager;
    [crate::MAIN_LABEL, crate::overlay::PANEL_LABEL]
        .iter()
        .any(|label| {
            app.get_webview_window(label)
                .is_some_and(|window| window.is_focused().unwrap_or(false))
        })
}

/// `alert` is the shell's `focus.alert=` value.
pub fn phase_ended(app: &tauri::AppHandle, alert: &str) {
    if in_front(app) {
        return;
    }
    let (title, body) = match alert {
        "finished" => (
            "Focus complete",
            "Your round is done. Shut sites are open again.",
        ),
        "break" => (
            "Time for a break",
            "Shut sites are open until the next round.",
        ),
        "focus" => ("Back to focus", "A new round has started."),
        _ => return,
    };
    notify("app.quasar.focus", title, body);
}
