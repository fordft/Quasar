//! Whether Zephium is the default browser, and asking to become it. Each
//! platform keeps the decision with the person: macOS confirms, Windows only
//! opens its Settings, Linux follows the desktop's own setting. The answer is
//! always read back rather than assumed.

use super::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::external_browser;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

/// Whether an application path is a Quasar build (Quasar.app, Quasar Dev.app).
/// A link handed to one would open inside the app that is signing in.
#[cfg(any(target_os = "macos", test))]
fn names_quasar_app(path: &str) -> bool {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("Quasar"))
}

#[cfg(test)]
mod tests {
    use super::names_quasar_app;

    #[test]
    fn recognises_every_quasar_build_but_no_other_browser() {
        for ours in [
            "/Applications/Quasar.app",
            "/Applications/Quasar Dev.app/",
            "/Users/me/Applications/Quasar.app",
        ] {
            assert!(names_quasar_app(ours), "{ours}");
        }
        for other in [
            "/Applications/Arc.app",
            "/Applications/Safari.app",
            "/Applications/Google Chrome.app",
            "/Applications/Firefox.app",
            "",
        ] {
            assert!(!names_quasar_app(other), "{other}");
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, PartialEq, Eq)]
pub(crate) struct DefaultBrowserStatus {
    pub(crate) is_default: bool,
    /// Whether asking can do anything here. False for a development build,
    /// which the system cannot register.
    pub(crate) can_request: bool,
}

fn status() -> DefaultBrowserStatus {
    #[cfg(target_os = "macos")]
    return DefaultBrowserStatus {
        is_default: macos::is_default(),
        can_request: macos::can_request(),
    };
    #[cfg(target_os = "windows")]
    return DefaultBrowserStatus {
        is_default: windows::is_default(),
        can_request: true,
    };
    #[cfg(target_os = "linux")]
    return DefaultBrowserStatus {
        is_default: linux::is_default(),
        can_request: linux::can_request(),
    };
    #[allow(unreachable_code)]
    DefaultBrowserStatus {
        is_default: false,
        can_request: false,
    }
}

/// Reading the answer can mean a registry walk or a child process, so it never
/// runs on the thread that serves the window.
async fn read_status() -> DefaultBrowserStatus {
    tauri::async_runtime::spawn_blocking(status)
        .await
        .unwrap_or(DefaultBrowserStatus {
            is_default: false,
            can_request: false,
        })
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn default_browser_status(caller: WebviewWindow) -> Option<DefaultBrowserStatus> {
    if !authorize(&caller, CallerPolicy::Main, "default_browser_status") {
        return None;
    }
    Some(read_status().await)
}

/// Asks the system to make Zephium the default browser, then reports what is
/// true afterwards. On Windows the answer arrives later, from Settings; chrome
/// reads the status again when the window regains focus.
#[tauri::command]
#[specta::specta]
pub(crate) async fn default_browser_request(caller: WebviewWindow) -> Option<DefaultBrowserStatus> {
    if !authorize(&caller, CallerPolicy::Main, "default_browser_request") {
        return None;
    }
    static ASKING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let Ok(_permit) = ASKING.try_acquire() else {
        return Some(read_status().await);
    };
    #[cfg(target_os = "macos")]
    {
        let (answered, answer) = tokio::sync::oneshot::channel();
        macos::request(Box::new(move || {
            let _ = answered.send(());
        }));
        // The person may leave the system prompt open; whatever is true by
        // then is reported, and focus brings a later answer back.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(120), answer).await;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = tauri::async_runtime::spawn_blocking(|| {
            if windows::register() {
                windows::open_settings();
            }
        })
        .await;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = tauri::async_runtime::spawn_blocking(linux::request).await;
    }
    Some(read_status().await)
}
