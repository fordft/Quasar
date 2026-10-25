//! Native ChatGPT authorization. The frame receives display state only.
use super::*;
use quasar_chatgpt::{Error, LoginOptions, Runtime, Store, SystemKey};

mod legacy;
static SERVICE: OnceLock<Arc<Runtime>> = OnceLock::new();
static ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();
static INITIALIZE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static INSTALL_FAULT: Mutex<Option<AccountFaultV1>> = Mutex::new(None);
static MIGRATION_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountFaultV1 {
    Offline,
    Browser,
    Protocol,
    InvalidIdentity,
    InvalidCallback,
    Denied,
    Cancelled,
    TimedOut,
    Busy,
    Vault,
    Unavailable,
    SignedOut,
    PlanPermissionRequired,
    UsageLimit,
    RateLimited,
    RetryLater,
    RemoteRevocationUnconfirmed,
}
impl From<Error> for AccountFaultV1 {
    fn from(error: Error) -> Self {
        match error {
            Error::Network => Self::Offline,
            Error::Protocol => Self::Protocol,
            Error::InvalidIdentity => Self::InvalidIdentity,
            Error::InvalidCallback => Self::InvalidCallback,
            Error::Denied => Self::Denied,
            Error::Cancelled => Self::Cancelled,
            Error::TimedOut => Self::TimedOut,
            Error::Busy => Self::Busy,
            Error::Storage => Self::Vault,
            Error::Unsupported => Self::Unavailable,
            Error::SignedOut => Self::SignedOut,
            Error::PlanPermissionRequired => Self::PlanPermissionRequired,
            Error::UsageLimit => Self::UsageLimit,
            Error::RateLimited => Self::RateLimited,
            Error::RetryLater => Self::RetryLater,
            Error::RemoteRevocationUnconfirmed => Self::RemoteRevocationUnconfirmed,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct AccountProfileV1 {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) email: Option<String>,
    pub(crate) signed_in: bool,
    pub(crate) sharing: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct AccountV1 {
    pub(crate) version: u32,
    pub(crate) available: bool,
    pub(crate) signed_in: bool,
    pub(crate) sharing: bool,
    pub(crate) pending: bool,
    pub(crate) active: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) profiles: Vec<AccountProfileV1>,
    pub(crate) migration_pending: bool,
    pub(crate) fault: Option<AccountFaultV1>,
}
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct AccountReplyV1 {
    pub(crate) account: AccountV1,
    pub(crate) fault: Option<AccountFaultV1>,
}
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[tauri_specta(event_name = "zephium:account-changed")]
pub(crate) struct AccountChanged {
    pub(crate) version: u32,
}

pub(crate) fn emit_changed(app: &tauri::AppHandle) {
    emit_to_privileged(
        app,
        MAIN_LABEL,
        "zephium:account-changed",
        &AccountChanged { version: 1 },
    );
}

fn changed(app: &tauri::AppHandle) {
    emit_changed(app);
    #[cfg(feature = "work-product")]
    zephium_app::work_models::session_changed();
}

pub(crate) fn install(app: &tauri::AppHandle, data_dir: &std::path::Path) {
    let _ = ROOT.set(data_dir.join("chatgpt"));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let cleanup = tauri::async_runtime::spawn_blocking(legacy::retire).await;
        MIGRATION_PENDING.store(!matches!(cleanup, Ok(Ok(()))), Ordering::Release);
        let _ = initialize(&app).await;
        changed(&app);
    });
}

async fn initialize(app: &tauri::AppHandle) -> Result<Arc<Runtime>, AccountFaultV1> {
    let _turn = INITIALIZE.lock().await;
    if let Some(runtime) = SERVICE.get() {
        return Ok(runtime.clone());
    }
    if !cfg!(any(target_os = "macos", target_os = "windows")) {
        return Err(AccountFaultV1::Unavailable);
    }
    let root = ROOT.get().cloned().ok_or(AccountFaultV1::Vault)?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        Runtime::new(Store::new(root, Arc::new(SystemKey))?)
    })
    .await;
    let runtime = match result {
        Ok(Ok(runtime)) => runtime,
        result => {
            let fault = match result {
                Ok(Err(error)) => error.into(),
                _ => AccountFaultV1::Vault,
            };
            *INSTALL_FAULT.lock().unwrap_or_else(|p| p.into_inner()) = Some(fault);
            return Err(fault);
        }
    };
    if SERVICE.set(runtime.clone()).is_ok() {
        #[cfg(feature = "work-product")]
        zephium_app::work_models::install_chatgpt(runtime.clone());
        let mut epochs = runtime.subscribe();
        let events = app.clone();
        tauri::async_runtime::spawn(async move {
            while epochs.changed().await.is_ok() {
                changed(&events);
            }
        });
    }
    *INSTALL_FAULT.lock().unwrap_or_else(|p| p.into_inner()) = None;
    Ok(runtime)
}

fn admitted(caller: &WebviewWindow, command: &str) -> Result<Arc<Runtime>, AccountFaultV1> {
    if !authorize(caller, CallerPolicy::Main, command) {
        return Err(AccountFaultV1::Unavailable);
    }
    SERVICE.get().cloned().ok_or(AccountFaultV1::Unavailable)
}
fn empty() -> AccountV1 {
    AccountV1 {
        version: 1,
        available: cfg!(any(target_os = "macos", target_os = "windows")),
        signed_in: false,
        sharing: false,
        pending: false,
        active: None,
        email: None,
        name: None,
        profiles: Vec::new(),
        migration_pending: MIGRATION_PENDING.load(Ordering::Acquire),
        fault: *INSTALL_FAULT.lock().unwrap_or_else(|p| p.into_inner()),
    }
}
async fn reply(result: Result<(), AccountFaultV1>) -> AccountReplyV1 {
    let mut account = empty();
    if let Some(runtime) = SERVICE.get() {
        match runtime.session().await {
            Ok(session) => {
                let active = session.active();
                account.signed_in = active.is_some_and(|p| p.signed_in);
                account.sharing = session.sharing();
                account.email = active.and_then(|p| p.email.clone());
                account.name = active.map(|p| p.label.clone());
                account.active = session.active;
                account.pending = session.pending;
                account.fault = session.fault.map(Into::into);
                account.profiles = session
                    .profiles
                    .into_iter()
                    .map(|p| AccountProfileV1 {
                        id: p.id,
                        label: p.label,
                        email: p.email,
                        signed_in: p.signed_in,
                        sharing: p.sharing,
                    })
                    .collect();
            }
            Err(error) => account.fault = Some(error.into()),
        }
    }
    AccountReplyV1 {
        fault: result.err().or(account.fault),
        account,
    }
}

/// Opens only a native-constructed authorization or fixed usage URL. Neither
/// the URL nor the optional ID-token hint passes through chrome or diagnostics.
async fn open_system(app: &tauri::AppHandle, url: String, external_auth: bool) -> bool {
    let (send, receive) = tokio::sync::oneshot::channel();
    if app
        .run_on_main_thread(move || {
            #[cfg(target_os = "macos")]
            let opened = {
                use objc2_app_kit::{
                    NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration,
                };
                use objc2_foundation::{NSArray, NSError, NSString, NSURL};
                if external_auth {
                    // A browser must not send OAuth URLs or ID-token hints to
                    // its own tab/chrome projections when it becomes default.
                    let workspace = NSWorkspace::sharedWorkspace();
                    let target = NSURL::URLWithString(&NSString::from_str(&url));
                    let browser = workspace.URLForApplicationWithBundleIdentifier(
                        &NSString::from_str("com.apple.Safari"),
                    );
                    let (Some(target), Some(browser)) = (target, browser) else {
                        let _ = send.send(false);
                        return;
                    };
                    let answer = std::sync::Mutex::new(Some(send));
                    let done = block2::RcBlock::new(
                        move |application: *mut NSRunningApplication, error: *mut NSError| {
                            if let Some(send) =
                                answer.lock().unwrap_or_else(|p| p.into_inner()).take()
                            {
                                let _ = send.send(!application.is_null() && error.is_null());
                            }
                        },
                    );
                    let configuration = NSWorkspaceOpenConfiguration::configuration();
                    configuration.setAddsToRecentItems(false);
                    workspace.openURLs_withApplicationAtURL_configuration_completionHandler(
                        &NSArray::from_retained_slice(&[target]),
                        &browser,
                        &configuration,
                        Some(&done),
                    );
                    return;
                }
                NSURL::URLWithString(&NSString::from_str(&url))
                    .is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url))
            };
            #[cfg(target_os = "windows")]
            let opened = {
                use windows::core::{w, PCWSTR};
                use windows::Win32::UI::Shell::ShellExecuteW;
                use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
                // The stock external browser keeps the OAuth flow outside
                // Quasar even when Quasar owns the http/https associations.
                let address = if external_auth {
                    format!("microsoft-edge:{url}")
                } else {
                    url
                };
                let wide: Vec<u16> = address.encode_utf16().chain(Some(0)).collect();
                // SAFETY: both strings are NUL terminated and remain alive for the call.
                unsafe {
                    ShellExecuteW(
                        None,
                        w!("open"),
                        PCWSTR(wide.as_ptr()),
                        None,
                        None,
                        SW_SHOWNORMAL,
                    )
                    .0 as isize
                        > 32
                }
            };
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let opened = {
                let _ = (url, external_auth);
                false
            };
            let _ = send.send(opened);
        })
        .is_err()
    {
        return false;
    }
    receive.await.unwrap_or(false)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn account_get(caller: WebviewWindow) -> AccountReplyV1 {
    // Never project account identities to an unprivileged caller.
    if !authorize(&caller, CallerPolicy::Main, "account_get") {
        return AccountReplyV1 {
            account: empty(),
            fault: Some(AccountFaultV1::Unavailable),
        };
    }
    reply(Ok(())).await
}
#[tauri::command]
#[specta::specta]
pub(crate) async fn account_sign_in(
    caller: WebviewWindow,
    profile_id: Option<String>,
    new_profile: bool,
    consent: bool,
) -> AccountReplyV1 {
    if !authorize(&caller, CallerPolicy::Main, "account_sign_in") {
        return AccountReplyV1 {
            account: empty(),
            fault: Some(AccountFaultV1::Unavailable),
        };
    }
    let runtime = match initialize(caller.app_handle()).await {
        Ok(runtime) => runtime,
        Err(fault) => return reply(Err(fault)).await,
    };
    if profile_id.as_ref().is_some_and(|id| id.len() > 128) {
        return reply(Err(AccountFaultV1::InvalidCallback)).await;
    }
    let authorization = match runtime
        .begin(LoginOptions {
            profile_id,
            new_profile,
            consent,
        })
        .await
    {
        Ok(a) => a,
        Err(e) => return reply(Err(e.into())).await,
    };
    let app = caller.app_handle().clone();
    if !open_system(&app, authorization.url().to_owned(), true).await {
        runtime.cancel();
        return AccountReplyV1 {
            account: empty(),
            fault: Some(AccountFaultV1::Unavailable),
        };
    }
    changed(&app);
    tauri::async_runtime::spawn(async move {
        let _ = runtime.finish(authorization).await;
        changed(&app);
        #[cfg(feature = "work-product")]
        zephium_app::work_models::refresh_catalog().await;
    });
    reply(Ok(())).await
}
#[tauri::command]
#[specta::specta]
pub(crate) async fn account_cancel_sign_in(caller: WebviewWindow) -> AccountReplyV1 {
    if !authorize(&caller, CallerPolicy::Main, "account_cancel_sign_in") {
        return AccountReplyV1 {
            account: empty(),
            fault: Some(AccountFaultV1::Unavailable),
        };
    }
    let result = admitted(&caller, "account_cancel_sign_in").map(|r| r.cancel());
    changed(caller.app_handle());
    reply(result).await
}
#[tauri::command]
#[specta::specta]
pub(crate) async fn account_select(caller: WebviewWindow, id: String) -> AccountReplyV1 {
    if !authorize(&caller, CallerPolicy::Main, "account_select") {
        return AccountReplyV1 {
            account: empty(),
            fault: Some(AccountFaultV1::Unavailable),
        };
    }
    let result = match admitted(&caller, "account_select") {
        Ok(r) if id.len() <= 128 => r.select(&id).await.map_err(Into::into),
        Ok(_) => Err(AccountFaultV1::InvalidCallback),
        Err(e) => Err(e),
    };
    changed(caller.app_handle());
    reply(result).await
}
#[tauri::command]
#[specta::specta]
pub(crate) async fn account_sign_out(caller: WebviewWindow) -> AccountReplyV1 {
    if !authorize(&caller, CallerPolicy::Main, "account_sign_out") {
        return AccountReplyV1 {
            account: empty(),
            fault: Some(AccountFaultV1::Unavailable),
        };
    }
    let result = match admitted(&caller, "account_sign_out") {
        Ok(r) => r.sign_out().await.map_err(Into::into),
        Err(e) => Err(e),
    };
    changed(caller.app_handle());
    reply(result).await
}
#[tauri::command]
#[specta::specta]
pub(crate) async fn account_manage_usage(caller: WebviewWindow) -> bool {
    if !authorize(&caller, CallerPolicy::Main, "account_manage_usage") {
        return false;
    }
    open_system(
        caller.app_handle(),
        quasar_chatgpt::USAGE_URL.to_owned(),
        false,
    )
    .await
}
#[tauri::command]
#[specta::specta]
pub(crate) async fn account_retry_cleanup(caller: WebviewWindow) -> AccountReplyV1 {
    if !authorize(&caller, CallerPolicy::Main, "account_retry_cleanup") {
        return AccountReplyV1 {
            account: empty(),
            fault: Some(AccountFaultV1::Unavailable),
        };
    }
    let result = tauri::async_runtime::spawn_blocking(legacy::retire).await;
    let pending = !matches!(result, Ok(Ok(())));
    MIGRATION_PENDING.store(pending, Ordering::Release);
    changed(caller.app_handle());
    reply(if pending {
        Err(AccountFaultV1::Vault)
    } else {
        Ok(())
    })
    .await
}
