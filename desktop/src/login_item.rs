//! Opening Zephium at login. macOS lists the app under Open at Login through
//! `SMAppService`; Windows keeps a per-user `Run` value. Either way the answer
//! is read back from the system, so a change made in its own settings shows.

use super::*;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, specta::Type, PartialEq, Eq)]
pub(crate) struct LoginItemStatus {
    /// False where this build cannot register, such as an unbundled binary.
    pub(crate) supported: bool,
    pub(crate) enabled: bool,
    /// Registered, but the person still has to allow it in System Settings.
    pub(crate) needs_approval: bool,
}

#[cfg(target_os = "macos")]
mod macos {
    use super::LoginItemStatus;
    use objc2_foundation::NSBundle;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    /// A development binary is not an app the system can list.
    fn bundled() -> bool {
        NSBundle::mainBundle()
            .bundleURL()
            .path()
            .is_some_and(|path| path.to_string().ends_with(".app"))
    }

    pub(super) fn status() -> LoginItemStatus {
        if !bundled() {
            return LoginItemStatus::default();
        }
        // SAFETY: a class method returning the process's own service object.
        let raw = unsafe { SMAppService::mainAppService().status() };
        super::from_service_status(raw.0)
    }

    pub(super) fn set(enabled: bool) {
        if !bundled() {
            return;
        }
        // SAFETY: the service is the main app's; errors are read back as status.
        unsafe {
            let service = SMAppService::mainAppService();
            let current = service.status();
            let _ = if enabled {
                service.registerAndReturnError()
            } else if current != SMAppServiceStatus::NotRegistered {
                service.unregisterAndReturnError()
            } else {
                Ok(())
            };
        }
    }

    pub(super) fn open_settings() {
        // SAFETY: opens System Settings › General › Login Items.
        unsafe { SMAppService::openSystemSettingsLoginItems() };
    }
}

/// `SMAppServiceStatus`: 0 not registered, 1 enabled, 2 requires approval,
/// 3 not found.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn from_service_status(raw: isize) -> LoginItemStatus {
    LoginItemStatus {
        supported: true,
        enabled: raw == 1,
        needs_approval: raw == 2,
    }
}

/// The `Run` value: the quoted executable, so a path with spaces launches.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn run_value(exe: &str) -> String {
    format!("\"{exe}\"")
}

/// Task Manager writes a 12-byte `StartupApproved\Run` record whose first
/// byte is even while the entry may run and odd once it is disabled.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn startup_approved(record: Option<&[u8]>) -> bool {
    record
        .and_then(|bytes| bytes.first())
        .is_none_or(|first| first % 2 == 0)
}

#[cfg(target_os = "windows")]
mod windows {
    use super::LoginItemStatus;
    use ::windows::core::PCWSTR;
    use ::windows::Win32::Foundation::ERROR_SUCCESS;
    use ::windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_WRITE, REG_BINARY, REG_OPTION_NON_VOLATILE, REG_SZ,
        RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
    };

    const NAME: &str = "Quasar";
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn read(
        key: &str,
        flags: ::windows::Win32::System::Registry::REG_ROUTINE_FLAGS,
    ) -> Option<Vec<u8>> {
        let key = wide(key);
        let name = wide(NAME);
        let mut buffer = [0u8; 1024];
        let mut size = buffer.len() as u32;
        // SAFETY: the buffer and its byte size describe the same live array.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
                flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        (status == ERROR_SUCCESS).then(|| buffer[..(size as usize).min(buffer.len())].to_vec())
    }

    fn write(
        key: &str,
        kind: ::windows::Win32::System::Registry::REG_VALUE_TYPE,
        data: &[u8],
    ) -> bool {
        let key = wide(key);
        let name = wide(NAME);
        let mut handle = HKEY::default();
        // SAFETY: pointers come from live buffers; the handle is closed below.
        unsafe {
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut handle,
                None,
            ) != ERROR_SUCCESS
            {
                return false;
            }
            let written = RegSetValueExW(handle, PCWSTR(name.as_ptr()), None, kind, Some(data));
            let _ = RegCloseKey(handle);
            written == ERROR_SUCCESS
        }
    }

    fn delete(key: &str) {
        let key = wide(key);
        let name = wide(NAME);
        // SAFETY: both strings outlive the call.
        let _ = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
            )
        };
    }

    fn exe() -> Option<String> {
        std::env::current_exe()
            .ok()
            .map(|path| path.display().to_string())
    }

    pub(super) fn status() -> LoginItemStatus {
        let Some(exe) = exe() else {
            return LoginItemStatus::default();
        };
        let registered = read(RUN, RRF_RT_REG_SZ).is_some_and(|bytes| {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|unit| *unit != 0)
                .collect();
            String::from_utf16_lossy(&units) == super::run_value(&exe)
        });
        let approved = super::startup_approved(read(APPROVED, RRF_RT_REG_BINARY).as_deref());
        LoginItemStatus {
            supported: true,
            enabled: registered && approved,
            needs_approval: false,
        }
    }

    pub(super) fn set(enabled: bool) {
        if !enabled {
            delete(RUN);
            return;
        }
        let Some(exe) = exe() else {
            return;
        };
        let data: Vec<u8> = wide(&super::run_value(&exe))
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        if write(RUN, REG_SZ, &data) {
            // Turning it on here overrides an earlier Task Manager disable.
            let mut record = [0u8; 12];
            record[0] = 2;
            let _ = write(APPROVED, REG_BINARY, &record);
        }
    }
}

fn status() -> LoginItemStatus {
    #[cfg(target_os = "macos")]
    return macos::status();
    #[cfg(target_os = "windows")]
    return windows::status();
    #[allow(unreachable_code)]
    LoginItemStatus::default()
}

#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    allow(unused_variables)
)]
fn set(enabled: bool) {
    #[cfg(target_os = "macos")]
    macos::set(enabled);
    #[cfg(target_os = "windows")]
    windows::set(enabled);
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn login_item_status(caller: WebviewWindow) -> LoginItemStatus {
    if !authorize(&caller, CallerPolicy::Main, "login_item_status") {
        return LoginItemStatus::default();
    }
    tauri::async_runtime::spawn_blocking(status)
        .await
        .unwrap_or_default()
}

/// Registers or removes the login item, then reports what the system holds.
#[tauri::command]
#[specta::specta]
pub(crate) async fn login_item_set(caller: WebviewWindow, enabled: bool) -> LoginItemStatus {
    if !authorize(&caller, CallerPolicy::Main, "login_item_set") {
        return LoginItemStatus::default();
    }
    tauri::async_runtime::spawn_blocking(move || {
        set(enabled);
        status()
    })
    .await
    .unwrap_or_default()
}

/// Opens Login Items in System Settings, where a pending approval is given.
#[tauri::command]
#[specta::specta]
pub(crate) fn login_item_open_settings(caller: WebviewWindow) -> bool {
    if !authorize(&caller, CallerPolicy::Main, "login_item_open_settings") {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_settings();
        return true;
    }
    #[allow(unreachable_code)]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_status_maps_to_on_off_and_approval() {
        let off = from_service_status(0);
        assert!(off.supported && !off.enabled && !off.needs_approval);
        let on = from_service_status(1);
        assert!(on.enabled && !on.needs_approval);
        let waiting = from_service_status(2);
        assert!(!waiting.enabled && waiting.needs_approval);
        let missing = from_service_status(3);
        assert!(!missing.enabled && !missing.needs_approval);
    }

    #[test]
    fn run_value_quotes_the_executable_and_approval_reads_task_manager() {
        assert_eq!(
            run_value(r"C:\Program Files\Quasar\Quasar.exe"),
            r#""C:\Program Files\Quasar\Quasar.exe""#
        );
        assert!(startup_approved(None));
        assert!(startup_approved(Some(&[2, 0, 0, 0])));
        assert!(startup_approved(Some(&[6, 0])));
        assert!(!startup_approved(Some(&[3, 0, 0, 0])));
        assert!(!startup_approved(Some(&[1])));
        assert!(startup_approved(Some(&[])));
    }
}
