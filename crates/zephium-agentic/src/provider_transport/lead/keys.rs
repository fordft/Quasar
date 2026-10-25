//! One login-Keychain generic password per provider. Secrets are read only to
//! authenticate a call; presence is answered from attributes alone.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zephium_core::work::model::{WorkModelError, WorkModelProvider};

use super::{LeadCredential, LeadSecret, LeadSecretFuture};

/// Shared with the development OpenAI credential and every older loader.
#[cfg(target_os = "macos")]
const ACCOUNT: &str = "development";

/// The Keychain service holding `provider`'s key, if it takes one.
pub fn service(provider: WorkModelProvider) -> Option<&'static str> {
    Some(match provider {
        WorkModelProvider::OpenAi => "app.quasar.agent-provider.openai",
        WorkModelProvider::Anthropic => "app.quasar.agent-provider.anthropic",
        WorkModelProvider::Google => "app.quasar.agent-provider.google",
        WorkModelProvider::DeepSeek => "app.quasar.agent-provider.deepseek",
        WorkModelProvider::OpenRouter => "app.quasar.agent-provider.openrouter",
        WorkModelProvider::XAi => "app.quasar.agent-provider.xai",
        WorkModelProvider::Mistral => "app.quasar.agent-provider.mistral",
        WorkModelProvider::Groq => "app.quasar.agent-provider.groq",
        WorkModelProvider::Cerebras => "app.quasar.agent-provider.cerebras",
        WorkModelProvider::Compatible => "app.quasar.agent-provider.compatible",
        WorkModelProvider::Cloud => return None,
    })
}

/// Content-free Keychain failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeadKeyError {
    /// No item for this provider.
    Missing,
    /// The Keychain refused or is unavailable.
    Inaccessible,
    /// The value is not a usable key.
    Invalid,
    /// An earlier Keychain call is waiting on a prompt the person has not
    /// answered; this one never reached the Keychain.
    AwaitingApproval,
}

fn cache() -> &'static Mutex<HashMap<WorkModelProvider, Arc<LeadSecret>>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<WorkModelProvider, Arc<LeadSecret>>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn forget(provider: WorkModelProvider) {
    if let Ok(mut cache) = cache().lock() {
        cache.remove(&provider);
    }
}

fn cached(provider: WorkModelProvider) -> Option<Arc<LeadSecret>> {
    cache().lock().ok()?.get(&provider).cloned()
}

/// Secret storage boundary. Tests supply memory implementations, never the OS vault.
pub trait KeyVault: Send + Sync {
    /// Read a key off the async executor.
    fn load(&self, provider: WorkModelProvider) -> Result<Arc<LeadSecret>, LeadKeyError>;
    /// Replace a key off the async executor.
    fn store(&self, provider: WorkModelProvider, secret: LeadSecret) -> Result<(), LeadKeyError>;
    /// Remove a key off the async executor.
    fn clear(&self, provider: WorkModelProvider) -> Result<(), LeadKeyError>;
    /// Read presence without reading the secret.
    fn present(&self, provider: WorkModelProvider) -> Result<bool, LeadKeyError>;
}

/// The operating system key vault used by the production app.
pub struct SystemVault;
impl KeyVault for SystemVault {
    fn load(&self, provider: WorkModelProvider) -> Result<Arc<LeadSecret>, LeadKeyError> {
        load(provider)
    }
    fn store(&self, provider: WorkModelProvider, secret: LeadSecret) -> Result<(), LeadKeyError> {
        store(provider, secret)
    }
    fn clear(&self, provider: WorkModelProvider) -> Result<(), LeadKeyError> {
        clear(provider)
    }
    fn present(&self, provider: WorkModelProvider) -> Result<bool, LeadKeyError> {
        present(provider)
    }
}

/// Loads `provider`'s key. Blocking; call off the async executor.
pub fn load(provider: WorkModelProvider) -> Result<Arc<LeadSecret>, LeadKeyError> {
    if let Some(secret) = cached(provider) {
        return Ok(secret);
    }
    let service = service(provider).ok_or(LeadKeyError::Missing)?;
    let secret = Arc::new(platform::read(service)?);
    if let Ok(mut cache) = cache().lock() {
        cache.insert(provider, secret.clone());
    }
    Ok(secret)
}

/// Stores `secret` as `provider`'s key, replacing any earlier one. Blocking.
pub fn store(provider: WorkModelProvider, secret: LeadSecret) -> Result<(), LeadKeyError> {
    let service = service(provider).ok_or(LeadKeyError::Invalid)?;
    forget(provider);
    platform::write(service, &secret)?;
    if let Ok(mut cache) = cache().lock() {
        cache.insert(provider, Arc::new(secret));
    }
    Ok(())
}

/// Removes `provider`'s key. Removing a missing key succeeds. Blocking.
pub fn clear(provider: WorkModelProvider) -> Result<(), LeadKeyError> {
    let service = service(provider).ok_or(LeadKeyError::Invalid)?;
    forget(provider);
    platform::delete(service)
}

/// Whether an item exists, from its attributes only (never its secret). Blocking.
pub fn present(provider: WorkModelProvider) -> Result<bool, LeadKeyError> {
    if cached(provider).is_some() {
        return Ok(true);
    }
    let service = service(provider).ok_or(LeadKeyError::Missing)?;
    platform::present(service)
}

/// Stores the separate TypeSafe/Jev decision credential at its existing fixed
/// native target. This never adds TypeSafe to the general model-provider list.
/// Blocking; the validated secret is zeroized when ownership ends.
pub fn store_typesafe(secret: LeadSecret) -> Result<(), LeadKeyError> {
    if secret.expose().len() > crate::MAX_AGENT_PROVIDER_CREDENTIAL_BYTES {
        return Err(LeadKeyError::Invalid);
    }
    platform::write("app.quasar.agent-provider.typesafe", &secret)
}

/// Removes only the fixed TypeSafe/Jev decision credential. Missing succeeds.
/// Blocking; no secret is read back to perform removal.
pub fn clear_typesafe() -> Result<(), LeadKeyError> {
    platform::delete("app.quasar.agent-provider.typesafe")
}

/// Reads the provider's Keychain key once per process, again after a refusal.
pub struct KeychainCredential(WorkModelProvider);

impl KeychainCredential {
    /// The key source for `provider`.
    pub fn new(provider: WorkModelProvider) -> Self {
        Self(provider)
    }
}

impl LeadCredential for KeychainCredential {
    fn secret(&self) -> LeadSecretFuture<'_> {
        let provider = self.0;
        Box::pin(async move {
            if let Some(secret) = cached(provider) {
                return Ok(secret);
            }
            match tokio::task::spawn_blocking(move || load(provider)).await {
                Ok(Ok(secret)) => Ok(secret),
                Ok(Err(LeadKeyError::Missing | LeadKeyError::Invalid)) => {
                    Err(WorkModelError::MissingKey)
                }
                // Not the provider refusing the key: the Keychain is busy.
                Ok(Err(LeadKeyError::AwaitingApproval)) => Err(WorkModelError::MissingKey),
                Ok(Err(LeadKeyError::Inaccessible)) | Err(_) => Err(WorkModelError::Unauthorized),
            }
        })
    }

    fn rejected(&self) {
        forget(self.0);
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use security_framework::item::{ItemClass, ItemSearchOptions};
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::os::macos::passwords::find_generic_password;
    use security_framework_sys::base::errSecItemNotFound;
    use zeroize::Zeroizing;

    use super::{LeadKeyError, LeadSecret, ACCOUNT};

    fn login() -> Result<SecKeychain, LeadKeyError> {
        let path = dirs::home_dir()
            .ok_or(LeadKeyError::Inaccessible)?
            .join("Library/Keychains/login.keychain-db");
        SecKeychain::open(path).map_err(|_| LeadKeyError::Inaccessible)
    }

    fn search(service: &str, keychain: &SecKeychain) -> ItemSearchOptions {
        let mut options = ItemSearchOptions::new();
        options
            .class(ItemClass::generic_password())
            .keychains(std::slice::from_ref(keychain))
            .service(service)
            .account(ACCOUNT);
        options
    }

    fn turn() -> Result<zephium_credentials::Turn, LeadKeyError> {
        crate::provider_transport::keychain_turn().map_err(|_| LeadKeyError::AwaitingApproval)
    }

    pub(super) fn read(service: &str) -> Result<LeadSecret, LeadKeyError> {
        let _turn = turn()?;
        match read_once(service) {
            Err(LeadKeyError::Inaccessible) => {
                std::thread::sleep(std::time::Duration::from_millis(40));
                read_once(service)
            }
            read => read,
        }
    }

    fn read_once(service: &str) -> Result<LeadSecret, LeadKeyError> {
        let keychain = login()?;
        let (password, _item) = find_generic_password(Some(&[keychain]), service, ACCOUNT)
            .map_err(|error| {
                if error.code() == errSecItemNotFound {
                    LeadKeyError::Missing
                } else {
                    LeadKeyError::Inaccessible
                }
            })?;
        let bytes = Zeroizing::new(password.as_ref().to_vec());
        let text = std::str::from_utf8(&bytes).map_err(|_| LeadKeyError::Invalid)?;
        LeadSecret::new(text.to_owned()).map_err(|_| LeadKeyError::Invalid)
    }

    pub(super) fn write(service: &str, secret: &LeadSecret) -> Result<(), LeadKeyError> {
        let _turn = turn()?;
        let keychain = login()?;
        // Replace by delete + add: updating in place would first read the
        // old secret, which prompts when another binary created the item.
        let _ = search(service, &keychain).delete();
        keychain
            .add_generic_password(service, ACCOUNT, secret.expose().as_bytes())
            .or_else(|_| {
                keychain.set_generic_password(service, ACCOUNT, secret.expose().as_bytes())
            })
            .map_err(|_| LeadKeyError::Inaccessible)
    }

    pub(super) fn delete(service: &str) -> Result<(), LeadKeyError> {
        let _turn = turn()?;
        let keychain = login()?;
        match search(service, &keychain).delete() {
            Ok(()) => Ok(()),
            Err(error) if error.code() == errSecItemNotFound => Ok(()),
            Err(_) => Err(LeadKeyError::Inaccessible),
        }
    }

    pub(super) fn present(service: &str) -> Result<bool, LeadKeyError> {
        let _turn = turn()?;
        let keychain = login()?;
        let mut options = search(service, &keychain);
        options.load_attributes(true);
        match options.search() {
            Ok(items) => Ok(!items.is_empty()),
            Err(error) if error.code() == errSecItemNotFound => Ok(false),
            Err(_) => Err(LeadKeyError::Inaccessible),
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{LeadKeyError, LeadSecret};
    use zephium_credentials::VaultError;

    fn map(error: VaultError) -> LeadKeyError {
        match error {
            VaultError::Missing => LeadKeyError::Missing,
            VaultError::Invalid => LeadKeyError::Invalid,
            VaultError::Inaccessible | VaultError::Capacity => LeadKeyError::Inaccessible,
            VaultError::AwaitingApproval => LeadKeyError::AwaitingApproval,
        }
    }

    pub(super) fn read(service: &str) -> Result<LeadSecret, LeadKeyError> {
        let _turn = crate::provider_transport::keychain_turn().map_err(map)?;
        let loaded = match zephium_credentials::read(service) {
            Err(VaultError::Inaccessible) => {
                std::thread::sleep(std::time::Duration::from_millis(40));
                zephium_credentials::read(service)
            }
            loaded => loaded,
        };
        let bytes = loaded.map_err(map)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| LeadKeyError::Invalid)?;
        LeadSecret::new(text.to_owned()).map_err(|_| LeadKeyError::Invalid)
    }

    pub(super) fn write(service: &str, secret: &LeadSecret) -> Result<(), LeadKeyError> {
        let _turn = crate::provider_transport::keychain_turn().map_err(map)?;
        zephium_credentials::write(service, secret.expose().as_bytes()).map_err(map)
    }

    pub(super) fn delete(service: &str) -> Result<(), LeadKeyError> {
        let _turn = crate::provider_transport::keychain_turn().map_err(map)?;
        zephium_credentials::delete(service).map_err(map)
    }

    pub(super) fn present(service: &str) -> Result<bool, LeadKeyError> {
        let _turn = crate::provider_transport::keychain_turn().map_err(map)?;
        zephium_credentials::present(service).map_err(map)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{LeadKeyError, LeadSecret};

    pub(super) fn read(_: &str) -> Result<LeadSecret, LeadKeyError> {
        Err(LeadKeyError::Missing)
    }
    pub(super) fn write(_: &str, _: &LeadSecret) -> Result<(), LeadKeyError> {
        Err(LeadKeyError::Inaccessible)
    }
    pub(super) fn delete(_: &str) -> Result<(), LeadKeyError> {
        Ok(())
    }
    pub(super) fn present(_: &str) -> Result<bool, LeadKeyError> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typesafe_setup_refuses_beyond_its_loader_bound_before_any_native_write() {
        let secret = LeadSecret::new("x".repeat(crate::MAX_AGENT_PROVIDER_CREDENTIAL_BYTES + 1))
            .expect("fits the general provider secret owner");
        assert_eq!(store_typesafe(secret), Err(LeadKeyError::Invalid));
        // This separate target does not alter the closed LLM provider mapping.
        assert_eq!(
            service(WorkModelProvider::OpenAi),
            Some("app.quasar.agent-provider.openai")
        );
        assert_eq!(service(WorkModelProvider::Cloud), None);
    }
}
