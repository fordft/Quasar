//! Native-only Sign in with ChatGPT. Credentials never cross the UI boundary.
#![forbid(unsafe_code)]

#[cfg(test)]
mod fixtures;
mod oauth;
mod runtime;
mod store;

pub use oauth::Authorization;
pub use runtime::{LoginOptions, Runtime};
pub use store::{EncryptionKey, Store, SystemKey};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The public API resource authorized by ChatGPT plan usage.
pub const RESOURCE: &str = "https://api.openai.com/v1";
/// The user controls the app's plan usage here.
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";
pub(crate) const ISSUER: &str = "https://auth.openai.com";
pub(crate) const MAX_TOKEN_BYTES: usize = 16 * 1024;
pub(crate) const MAX_PROFILES: usize = 8;

/// Closed, content-free errors; never include OAuth codes or server bodies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    Network,
    Protocol,
    InvalidIdentity,
    InvalidCallback,
    Denied,
    Cancelled,
    TimedOut,
    Busy,
    Storage,
    Unsupported,
    SignedOut,
    PlanPermissionRequired,
    UsageLimit,
    RateLimited,
    RemoteRevocationUnconfirmed,
}

/// A bearer credential whose Debug representation is always redacted.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub(crate) fn new(value: String) -> Result<Self, Error> {
        if value.is_empty()
            || value.len() > MAX_TOKEN_BYTES
            || !value.bytes().all(|b| matches!(b, 0x21..=0x7e))
        {
            return Err(Error::Protocol);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    /// Only trusted native transport code may read a bearer.
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// Verified OIDC identity, separate from account registration and permissions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Identity {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// Safe display projection. It contains neither tokens nor authorization URLs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileView {
    pub id: String,
    pub label: String,
    pub email: Option<String>,
    pub signed_in: bool,
    pub sharing: bool,
}

/// The only account state exposed to application UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionView {
    pub active: Option<String>,
    pub profiles: Vec<ProfileView>,
    pub pending: bool,
    pub fault: Option<Error>,
}

impl SessionView {
    pub fn active(&self) -> Option<&ProfileView> {
        let active = self.active.as_ref()?;
        self.profiles.iter().find(|p| &p.id == active)
    }

    pub fn sharing(&self) -> bool {
        self.fault != Some(Error::PlanPermissionRequired)
            && self.active().is_some_and(|p| p.signed_in && p.sharing)
    }
}

/// Model identities come from the selected account's authenticated catalog.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub slug: String,
    pub display_name: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Tokens {
    pub access: Zeroizing<String>,
    pub refresh: Option<Zeroizing<String>>,
    pub id: Zeroizing<String>,
    pub scopes: Vec<String>,
    pub expires_at: u64,
    pub earliest_refresh_at: Option<u64>,
}

impl Tokens {
    pub(crate) fn sharing(&self) -> bool {
        ["resource.invoke", "chatgpt.tokens.use.direct"]
            .iter()
            .all(|wanted| self.scopes.iter().any(|scope| scope == wanted))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Profile {
    pub id: String,
    pub client_id: String,
    pub identity: Option<Identity>,
    pub tokens: Option<Tokens>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct State {
    pub version: u8,
    pub host_id: String,
    pub active: Option<String>,
    pub profiles: Vec<Profile>,
}

pub(crate) fn random<const N: usize>() -> Result<[u8; N], Error> {
    use ring::rand::SecureRandom;
    let mut bytes = [0; N];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| Error::Storage)?;
    Ok(bytes)
}

pub(crate) fn now() -> Result<u64, Error> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| Error::Protocol)
}

impl State {
    fn fresh() -> Result<Self, Error> {
        let mut bytes = random::<16>()?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex: String = bytes.iter().map(|v| format!("{v:02x}")).collect();
        Ok(Self {
            version: 1,
            host_id: format!(
                "urn:uuid:{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            ),
            active: None,
            profiles: Vec::new(),
        })
    }

    fn view(&self, pending: bool, fault: Option<Error>) -> SessionView {
        SessionView {
            active: self.active.clone(),
            profiles: self
                .profiles
                .iter()
                .enumerate()
                .map(|(index, p)| ProfileView {
                    id: p.id.clone(),
                    label: format!("ChatGPT connection {}", index + 1),
                    email: p.identity.as_ref().and_then(|i| i.email.clone()),
                    signed_in: p.tokens.is_some(),
                    sharing: p.tokens.as_ref().is_some_and(Tokens::sharing),
                })
                .collect(),
            pending,
            fault,
        }
    }
}
