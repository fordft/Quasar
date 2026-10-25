//! The session bearer's home in the OS vault: one item per account server,
//! shared by every profile. A bearer is only ever read for the origin that
//! issued it.

use crate::AccountToken;

/// Content-free vault failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultFault {
    Missing,
    Inaccessible,
}

/// Secret storage boundary, keyed by the exact API origin. Tests supply
/// memory vaults, never the OS one.
pub trait AccountVault: Send + Sync {
    fn load(&self, origin: &str) -> Result<AccountToken, VaultFault>;
    fn store(&self, origin: &str, token: &AccountToken) -> Result<(), VaultFault>;
    fn clear(&self, origin: &str) -> Result<(), VaultFault>;
    /// Presence from attributes alone, without reading the secret.
    fn present(&self, origin: &str) -> Result<bool, VaultFault>;
}

/// The Keychain on macOS and Credential Manager on Windows.
pub struct SystemVault;

/// The item every origin shared before bearers were scoped to one.
const UNSCOPED: &str = "app.quasar.account";

#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn item(origin: &str) -> String {
    format!("{UNSCOPED}|{origin}")
}

impl SystemVault {
    /// Removes the bearer kept before scoping. Its origin was never recorded,
    /// so it cannot be trusted to any one server and is never read.
    pub fn drop_unscoped(&self) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let Ok(_turn) = zephium_credentials::turn() else {
                return;
            };
            if zephium_credentials::present(UNSCOPED).unwrap_or(false) {
                let _ = zephium_credentials::delete(UNSCOPED);
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use super::{item, AccountToken, AccountVault, SystemVault, VaultFault};
    use zephium_credentials::VaultError;

    fn fault(error: VaultError) -> VaultFault {
        match error {
            VaultError::Missing => VaultFault::Missing,
            _ => VaultFault::Inaccessible,
        }
    }

    impl AccountVault for SystemVault {
        fn load(&self, origin: &str) -> Result<AccountToken, VaultFault> {
            let service = item(origin);
            let _turn = zephium_credentials::turn().map_err(fault)?;
            let bytes = match zephium_credentials::read(&service) {
                Err(VaultError::Inaccessible) => {
                    // Concurrent login-keychain reads fail transiently.
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    zephium_credentials::read(&service)
                }
                read => read,
            }
            .map_err(fault)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| VaultFault::Inaccessible)?;
            AccountToken::new(text.to_owned()).map_err(|_| VaultFault::Inaccessible)
        }

        fn store(&self, origin: &str, token: &AccountToken) -> Result<(), VaultFault> {
            let _turn = zephium_credentials::turn().map_err(fault)?;
            zephium_credentials::write(&item(origin), token.expose().as_bytes()).map_err(fault)
        }

        fn clear(&self, origin: &str) -> Result<(), VaultFault> {
            let _turn = zephium_credentials::turn().map_err(fault)?;
            match zephium_credentials::delete(&item(origin)) {
                Err(VaultError::Missing) => Ok(()),
                other => other.map_err(fault),
            }
        }

        fn present(&self, origin: &str) -> Result<bool, VaultFault> {
            let _turn = zephium_credentials::turn().map_err(fault)?;
            zephium_credentials::present(&item(origin)).map_err(fault)
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl AccountVault for SystemVault {
    fn load(&self, _: &str) -> Result<AccountToken, VaultFault> {
        Err(VaultFault::Missing)
    }
    fn store(&self, _: &str, _: &AccountToken) -> Result<(), VaultFault> {
        Err(VaultFault::Inaccessible)
    }
    fn clear(&self, _: &str) -> Result<(), VaultFault> {
        Ok(())
    }
    fn present(&self, _: &str) -> Result<bool, VaultFault> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_origin_has_its_own_item() {
        assert_eq!(
            super::item(crate::DEFAULT_ORIGIN),
            "app.quasar.account|https://api.zephium.app"
        );
        assert_eq!(
            super::item("http://127.0.0.1:8787"),
            "app.quasar.account|http://127.0.0.1:8787"
        );
    }
}
