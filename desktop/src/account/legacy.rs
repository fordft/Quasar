//! Idempotent retirement of AI credentials only. No network calls or changes
//! to website passwords, MCP credentials, browsing data or Work history.
use super::*;

const MARKER: &str = "quasar.chatgpt.legacy_retired.v1";
const SETTING_KEYS: &[&str] = &[
    "account.server",
    "account.cache",
    "account.plan_change",
    "account.call_tasks",
    "account.vault.scoped",
    "work.models.cloud.catalog",
    "work.models.compatible.base",
    "voice.engine",
];

pub(super) fn retire() -> Result<(), ()> {
    use zephium_core::ports::store::Store;
    let store = APP_STORE.get().ok_or(())?;
    if store.app_setting(MARKER).as_deref() == Some("1") {
        return Ok(());
    }
    // Read the old custom origin before retiring its setting. Never contact it.
    let custom = store.app_setting("account.server");
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let _turn = zephium_credentials::turn().map_err(|_| ())?;
        for prefix in ["app.quasar.account", "app.zephium.account"] {
            let mut services = vec![
                prefix.to_owned(),
                format!("{prefix}|https://api.zephium.app"),
            ];
            if let Some(origin) = custom.as_ref().filter(|s| s.len() <= 512) {
                if let Ok(url) = url::Url::parse(origin) {
                    if url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                    {
                        services.push(format!("{prefix}|{}", url.origin().ascii_serialization()));
                    }
                }
            }
            for service in services {
                match zephium_credentials::delete(&service) {
                    Ok(()) | Err(zephium_credentials::VaultError::Missing) => {}
                    Err(_) => return Err(()),
                }
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = custom;
    #[cfg(feature = "work-product")]
    zephium_agentic::lead::keys::retire_legacy_keys().map_err(|_| ())?;
    for key in SETTING_KEYS {
        if store.app_setting(key).is_some() && !store.set_app_setting((*key).into(), String::new())
        {
            return Err(());
        }
    }
    for slug in [
        "openai",
        "anthropic",
        "google",
        "deepseek",
        "openrouter",
        "xai",
        "mistral",
        "groq",
        "cerebras",
        "compatible",
    ] {
        let key = format!("work.models.key.{slug}");
        if store.app_setting(&key).is_some() && !store.set_app_setting(key, String::new()) {
            return Err(());
        }
    }
    if !store.set_app_setting(MARKER.into(), "1".into()) {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_is_scoped_to_ai_settings() {
        assert!(!SETTING_KEYS.iter().any(|key| key.starts_with("mcp")
            || key.starts_with("password")
            || key.starts_with("browser")));
        assert!(!SETTING_KEYS
            .iter()
            .any(|key| key.starts_with("quasar.chatgpt")));
    }
}
