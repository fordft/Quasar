//! Optional fixed-endpoint HTTPS transport for bounded agent model calls.
//!
//! The semantic core constructs and policy-admits every byte before this shell
//! sees it. This crate adds only provider credentials, exact HTTPS endpoints,
//! bounded concurrency, cancellation, response framing, and the existing
//! provider-neutral stream decoder. It exposes no arbitrary URL, generic HTTP
//! request, provider-native browser tool, raw response body, or generation retry.

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(clippy::dbg_macro, clippy::print_stderr, clippy::print_stdout)]
#![cfg_attr(
    not(test),
    deny(clippy::panic, clippy::unreachable, clippy::unwrap_used)
)]

/// Bounded objective/context planning over the shared fixed-endpoint transport.
pub mod agent;
/// Public search on the person's own engine's results page, with no model.
pub mod browser_search;
/// Typed decisions over admitted, enumerated questions.
pub mod decision;
/// General tool-calling transports for the lead agent.
pub mod lead;
/// Page-agent turns on the person's chosen model.
pub mod model_client;
/// Public search through the chosen model's own provider search.
pub mod model_search;
pub mod planning;
mod rig;
/// Bounded non-reasoning provider-native public search.
pub mod search;
/// Bounded semantic artifacts from admitted dependency context.
pub mod synthesis;
/// A pasted YouTube link's title and channel from YouTube's oEmbed endpoint.
pub mod youtube_oembed;

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::AgentProviderPricingSettlementError;
use crate::{
    AgentActiveModelCall, AgentCommittedProviderInput, AgentCommittedProviderRequest,
    AgentModelCallReceipt, AgentModelCallSettlement, AgentModelCallUnaccountedSettlement,
    AgentModelUsageAccounting, AgentPolicyError, AgentProviderCallConfig,
    AgentProviderCallIdentity, AgentProviderContinuationSeed, AgentProviderEndpoint,
    AgentProviderExactInputCount, AgentProviderFailure, AgentProviderFailureClass,
    AgentProviderFinishedStream, AgentProviderInputEvidence, AgentProviderInputMetricReceipt,
    AgentProviderInputMetrics, AgentProviderKind, AgentProviderPricingSettlement,
    AgentProviderRequestError, AgentProviderRetryAfter, AgentProviderStopReason,
    AgentProviderStreamBatch, AgentProviderStreamConclusion, AgentProviderStreamDecoder,
    AgentProviderTransportInput, AgentProviderUsage, AgentRunPolicy,
};
use base64::Engine as _;
use futures_util::TryStreamExt;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CACHE_CONTROL,
    CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER,
};
use reqwest::redirect::Policy;
use reqwest::{Client, StatusCode, Url};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio::sync::Notify;
use zeroize::Zeroizing;

/// Maximum secret bytes accepted for one provider credential.
pub const MAX_AGENT_PROVIDER_CREDENTIAL_BYTES: usize = 1_024;
/// Maximum simultaneous admitted HTTP attempts in one transport instance.
pub const MAX_AGENT_PROVIDER_TRANSPORT_CALLS: usize = 4;
/// Maximum total request lifetime accepted by transport configuration.
pub const MAX_AGENT_PROVIDER_REQUEST_TIMEOUT_MILLIS: u64 = 10 * 60 * 1_000;
/// Maximum connection-establishment deadline accepted by configuration.
pub const MAX_AGENT_PROVIDER_CONNECT_TIMEOUT_MILLIS: u64 = 30 * 1_000;
/// Maximum interval without response-body progress accepted by configuration.
pub const MAX_AGENT_PROVIDER_READ_TIMEOUT_MILLIS: u64 = 2 * 60 * 1_000;
/// Maximum decoded response-header list bytes accepted from a provider.
pub const MAX_AGENT_PROVIDER_RESPONSE_HEADER_BYTES: u32 = 64 * 1_024;
/// Initial per-stream and per-connection HTTP/2 receive credit.
pub const AGENT_PROVIDER_HTTP2_INITIAL_RECEIVE_WINDOW_BYTES: u32 = 65_535;
/// Maximum HTTP/2 frame size accepted by the provider client.
pub const MAX_AGENT_PROVIDER_HTTP2_FRAME_BYTES: u32 = 16 * 1_024;
/// Per-field accounting overhead from the HTTP/2 header-list size definition.
const HTTP_HEADER_FIELD_OVERHEAD_BYTES: usize = 32;
/// Maximum byte width of one provider-transport shutdown proof.
pub const MAX_AGENT_PROVIDER_TRANSPORT_SHUTDOWN_PROOF_BYTES: usize = 32;
/// Maximum JSON response bytes accepted from OpenAI input-token accounting.
pub const MAX_OPENAI_INPUT_TOKEN_RESPONSE_BYTES: usize = 1_024;

const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const OPENAI_INPUT_TOKENS_URL: &str = "https://api.openai.com/v1/responses/input_tokens";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
pub(crate) const PRODUCT_USER_AGENT: &str = "Quasar-Agent-Browser/0.1";
const OPENAI_CLIENT_REQUEST_ID_HEADER: &str = "x-client-request-id";
const OPENAI_CLIENT_REQUEST_ID_DOMAIN: &[u8] = b"ZEPHIUM-OPENAI-CLIENT-REQUEST-ID-1\0";

/// Login-Keychain service holding the development OpenAI provider credential.
#[cfg(target_os = "macos")]
pub const MACOS_OPENAI_KEYCHAIN_SERVICE: &str = "app.quasar.agent-provider.openai";
/// Login-Keychain account holding the development OpenAI provider credential.
#[cfg(target_os = "macos")]
pub const MACOS_OPENAI_KEYCHAIN_ACCOUNT: &str = "development";

/// Bounded deadlines for the shared provider client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentProviderTransportConfig {
    request_timeout: Duration,
    connect_timeout: Duration,
    read_timeout: Duration,
}

impl AgentProviderTransportConfig {
    /// Conservative production defaults with an idle-body progress deadline.
    pub const STANDARD: Self = Self {
        request_timeout: Duration::from_secs(5 * 60),
        connect_timeout: Duration::from_secs(15),
        read_timeout: Duration::from_secs(60),
    };

    /// Validates nonzero, ordered deadlines against product hard ceilings.
    pub fn try_new(
        request_timeout: Duration,
        connect_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Self, AgentProviderTransportConfigError> {
        if request_timeout.is_zero()
            || connect_timeout.is_zero()
            || read_timeout.is_zero()
            || connect_timeout > request_timeout
            || read_timeout > request_timeout
            || request_timeout.as_millis() > u128::from(MAX_AGENT_PROVIDER_REQUEST_TIMEOUT_MILLIS)
            || connect_timeout.as_millis() > u128::from(MAX_AGENT_PROVIDER_CONNECT_TIMEOUT_MILLIS)
            || read_timeout.as_millis() > u128::from(MAX_AGENT_PROVIDER_READ_TIMEOUT_MILLIS)
        {
            return Err(AgentProviderTransportConfigError::Deadlines);
        }
        Ok(Self {
            request_timeout,
            connect_timeout,
            read_timeout,
        })
    }

    /// End-to-end attempt deadline spanning admission, counting, and model streaming.
    pub const fn request_timeout(self) -> Duration {
        self.request_timeout
    }

    /// Connection-establishment deadline.
    pub const fn connect_timeout(self) -> Duration {
        self.connect_timeout
    }

    /// Maximum interval without response-body progress.
    pub const fn read_timeout(self) -> Duration {
        self.read_timeout
    }
}

/// Failure to construct the fixed provider client.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AgentProviderTransportConfigError {
    /// One or more deadlines were empty, contradictory, or above a hard bound.
    #[error("agent provider transport deadlines are invalid")]
    Deadlines,
    /// A compile-time provider endpoint was not an exact absolute URL.
    #[error("agent provider endpoint configuration is invalid")]
    Endpoint,
    /// The redirect-free HTTPS client could not be constructed.
    #[error("agent provider HTTPS client is unavailable")]
    Client,
}

/// Move-only, provider-bound credential whose owned bytes are zeroized on drop.
#[must_use]
pub struct AgentProviderCredential<P: AgentCredentialBinding = AgentProviderKind> {
    provider: P,
    secret: Zeroizing<Vec<u8>>,
}

mod credential_binding {
    pub trait Sealed {}
    impl Sealed for crate::AgentProviderKind {}
    impl Sealed for super::DecisionCredentialProvider {}
}

/// Closed provider identity carried by the shared zeroizing credential owner.
pub trait AgentCredentialBinding: credential_binding::Sealed + Copy + fmt::Debug {}
impl AgentCredentialBinding for AgentProviderKind {}
impl AgentCredentialBinding for DecisionCredentialProvider {}

/// Separate decision protocol identities; these cannot authenticate LLM transports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionCredentialProvider {
    /// Direct TypeSafe BYOK endpoint.
    TypeSafe,
    /// Session bearer for the Zephium decision proxy.
    ZephiumCloud,
}

impl<P: AgentCredentialBinding> AgentProviderCredential<P> {
    /// Consumes and validates one visible-ASCII provider secret.
    pub fn try_new(provider: P, secret: String) -> Result<Self, AgentProviderCredentialError> {
        Self::try_from_bytes(provider, secret.into_bytes())
    }

    fn try_from_bytes(provider: P, secret: Vec<u8>) -> Result<Self, AgentProviderCredentialError> {
        Self::try_from_zeroizing(provider, Zeroizing::new(secret))
    }

    fn try_from_zeroizing(
        provider: P,
        secret: Zeroizing<Vec<u8>>,
    ) -> Result<Self, AgentProviderCredentialError> {
        if secret.is_empty()
            || secret.len() > MAX_AGENT_PROVIDER_CREDENTIAL_BYTES
            || !secret.iter().all(|byte| matches!(byte, 0x21..=0x7e))
        {
            return Err(AgentProviderCredentialError::Content);
        }
        Ok(Self { provider, secret })
    }

    /// Exact provider protocol this credential may authenticate.
    pub const fn provider(&self) -> P {
        self.provider
    }

    /// Credential byte count without exposing the secret.
    pub fn byte_len(&self) -> usize {
        self.secret.len()
    }
}

/// Content-free failure while loading one provider credential from the OS vault.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AgentProviderVaultError {
    /// No exact generic-password item exists for the fixed service and account.
    #[error("agent provider credential is missing from the OS vault")]
    Missing,
    /// Keychain access was denied, unavailable, or otherwise failed closed.
    #[error("agent provider credential is inaccessible in the OS vault")]
    Inaccessible,
    /// The stored secret failed the provider credential content contract.
    #[error("agent provider credential stored in the OS vault is invalid")]
    Invalid,
    /// A bounded zeroizing credential copy could not be allocated.
    #[error("agent provider credential memory is unavailable")]
    Capacity,
    /// An earlier vault call is waiting on an OS prompt the person has not
    /// answered; this load never reached the vault.
    #[error("the OS vault is waiting for the person's approval of an earlier request")]
    AwaitingApproval,
}

/// Compatibility name for the macOS credential loader's closed errors.
#[cfg(target_os = "macos")]
pub type MacosAgentProviderCredentialError = AgentProviderVaultError;

/// Loads the fixed OpenAI credential through the platform's native vault.
pub fn load_development_openai_credential(
) -> Result<AgentProviderCredential, AgentProviderVaultError> {
    #[cfg(target_os = "macos")]
    {
        load_macos_development_openai_credential()
    }
    #[cfg(target_os = "windows")]
    {
        load_windows_credential(
            AgentProviderKind::OpenAiResponses,
            "app.quasar.agent-provider.openai",
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(AgentProviderVaultError::Inaccessible)
    }
}

/// Loads the fixed TypeSafe credential through the platform's native vault.
pub fn load_development_typesafe_credential(
) -> Result<AgentProviderCredential<DecisionCredentialProvider>, AgentProviderVaultError> {
    #[cfg(target_os = "macos")]
    {
        load_macos_development_typesafe_credential()
    }
    #[cfg(target_os = "windows")]
    {
        load_windows_credential(
            DecisionCredentialProvider::TypeSafe,
            "app.quasar.agent-provider.typesafe",
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(AgentProviderVaultError::Inaccessible)
    }
}

#[cfg(target_os = "windows")]
fn load_windows_credential<P: AgentCredentialBinding>(
    provider: P,
    target: &str,
) -> Result<AgentProviderCredential<P>, AgentProviderVaultError> {
    let _turn = keychain_turn().map_err(vault_error)?;
    let read = || zephium_credentials::read(target);
    let loaded = match read() {
        Err(zephium_credentials::VaultError::Inaccessible) => {
            std::thread::sleep(Duration::from_millis(40));
            read()
        }
        loaded => loaded,
    };
    let secret = loaded.map_err(vault_error)?;
    AgentProviderCredential::try_from_zeroizing(provider, secret).map_err(|error| match error {
        AgentProviderCredentialError::Content => AgentProviderVaultError::Invalid,
        AgentProviderCredentialError::Capacity => AgentProviderVaultError::Capacity,
    })
}

/// Loads the native OpenAI key for the release-excluded probe.
#[cfg(feature = "probe-harness")]
pub fn load_probe_openai_credential() -> Result<AgentProviderCredential, AgentProviderVaultError> {
    #[cfg(target_os = "macos")]
    let loaded = load_macos_probe_openai_credential();
    #[cfg(not(target_os = "macos"))]
    let loaded = load_development_openai_credential();
    loaded
}

/// Loads the exact development OpenAI key into a move-only zeroizing credential.
///
/// The Keychain lookup uses a fixed generic-password service and account. It
/// never enumerates items, converts secret bytes through UTF-8, accepts a
/// caller-selected label, or exposes the secret through an error or diagnostic.
/// The Security.framework-owned password buffer is released immediately after
/// one bounded copy enters [`AgentProviderCredential`].
#[cfg(target_os = "macos")]
pub fn load_macos_development_openai_credential(
) -> Result<AgentProviderCredential, MacosAgentProviderCredentialError> {
    load_keychain_login_credential(
        AgentProviderKind::OpenAiResponses,
        MACOS_OPENAI_KEYCHAIN_SERVICE,
        MACOS_OPENAI_KEYCHAIN_ACCOUNT,
    )
}

/// Fixed TypeSafe item in the person's login Keychain.
#[cfg(target_os = "macos")]
pub const MACOS_TYPESAFE_KEYCHAIN_SERVICE: &str = "app.quasar.agent-provider.typesafe";
/// Fixed development account for TypeSafe BYOK.
#[cfg(target_os = "macos")]
pub const MACOS_TYPESAFE_KEYCHAIN_ACCOUNT: &str = "development";

/// Loads TypeSafe BYOK into the same provider-bound zeroizing owner as OpenAI.
/// Parallel page reads load it at the same moment, and a read whose load
/// failed ran on the paid emulation for its whole life: loads take turns,
/// and an inaccessible Keychain is asked once more before falling back.
#[cfg(target_os = "macos")]
pub fn load_macos_development_typesafe_credential(
) -> Result<AgentProviderCredential<DecisionCredentialProvider>, MacosAgentProviderCredentialError>
{
    static LOADS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = LOADS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let load = || {
        load_keychain_login_credential(
            DecisionCredentialProvider::TypeSafe,
            MACOS_TYPESAFE_KEYCHAIN_SERVICE,
            MACOS_TYPESAFE_KEYCHAIN_ACCOUNT,
        )
    };
    match load() {
        Err(MacosAgentProviderCredentialError::Inaccessible) => load(),
        loaded => loaded,
    }
}

/// One Keychain call at a time across the process: concurrent reads of the
/// login keychain fail transiently, so every caller, including the account
/// session, takes the same turn. A call stuck behind an unanswered OS prompt
/// fails the ones after it fast instead of queueing them forever.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn keychain_turn() -> Result<zephium_credentials::Turn, zephium_credentials::VaultError>
{
    zephium_credentials::turn()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
const fn vault_error(error: zephium_credentials::VaultError) -> AgentProviderVaultError {
    match error {
        zephium_credentials::VaultError::Missing => AgentProviderVaultError::Missing,
        zephium_credentials::VaultError::Inaccessible => AgentProviderVaultError::Inaccessible,
        zephium_credentials::VaultError::Invalid => AgentProviderVaultError::Invalid,
        zephium_credentials::VaultError::Capacity => AgentProviderVaultError::Capacity,
        zephium_credentials::VaultError::AwaitingApproval => {
            AgentProviderVaultError::AwaitingApproval
        }
    }
}

#[cfg(target_os = "macos")]
fn load_keychain_login_credential<P: AgentCredentialBinding>(
    provider: P,
    service: &'static str,
    account: &'static str,
) -> Result<AgentProviderCredential<P>, MacosAgentProviderCredentialError> {
    let _turn = keychain_turn().map_err(vault_error)?;
    match read_keychain_login_credential(provider, service, account) {
        Err(MacosAgentProviderCredentialError::Inaccessible) => {
            std::thread::sleep(std::time::Duration::from_millis(40));
            read_keychain_login_credential(provider, service, account)
        }
        loaded => loaded,
    }
}

#[cfg(target_os = "macos")]
fn read_keychain_login_credential<P: AgentCredentialBinding>(
    provider: P,
    service: &'static str,
    account: &'static str,
) -> Result<AgentProviderCredential<P>, MacosAgentProviderCredentialError> {
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::os::macos::passwords::find_generic_password;
    use security_framework_sys::base::errSecItemNotFound;

    let login_path = dirs::home_dir()
        .ok_or(MacosAgentProviderCredentialError::Inaccessible)?
        .join("Library/Keychains/login.keychain-db");
    let login = SecKeychain::open(login_path)
        .map_err(|_| MacosAgentProviderCredentialError::Inaccessible)?;
    let (password, _item) =
        find_generic_password(Some(&[login]), service, account).map_err(|error| {
            if error.code() == errSecItemNotFound {
                MacosAgentProviderCredentialError::Missing
            } else {
                MacosAgentProviderCredentialError::Inaccessible
            }
        })?;
    let mut secret = Zeroizing::new(Vec::new());
    secret
        .try_reserve_exact(password.len())
        .map_err(|_| MacosAgentProviderCredentialError::Capacity)?;
    secret.extend_from_slice(password.as_ref());
    AgentProviderCredential::try_from_zeroizing(provider, secret).map_err(|error| match error {
        AgentProviderCredentialError::Content => MacosAgentProviderCredentialError::Invalid,
        AgentProviderCredentialError::Capacity => MacosAgentProviderCredentialError::Capacity,
    })
}

/// Uses the signed probe's own Keychain identity, just like the development app.
#[cfg(all(target_os = "macos", feature = "probe-harness"))]
pub fn load_macos_probe_openai_credential(
) -> Result<AgentProviderCredential, MacosAgentProviderCredentialError> {
    load_macos_development_openai_credential()
}

impl<P: AgentCredentialBinding> fmt::Debug for AgentProviderCredential<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderCredential")
            .field("provider", &self.provider)
            .field("bytes", &self.secret.len())
            .field("secret", &"[redacted]")
            .finish()
    }
}

/// Refusal to retain a provider credential.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AgentProviderCredentialError {
    /// Credential was empty, oversized, or not visible ASCII.
    #[error("agent provider credential is invalid")]
    Content,
    /// Memory for a bounded zeroizing credential copy was unavailable.
    #[error("agent provider credential memory is unavailable")]
    Capacity,
}

struct AgentProviderAttemptCredential {
    secret: Zeroizing<Vec<u8>>,
}

impl AgentProviderAttemptCredential {
    fn try_from_credential(
        credential: &AgentProviderCredential,
    ) -> Result<Self, AgentProviderCredentialError> {
        let mut secret = Zeroizing::new(Vec::new());
        secret
            .try_reserve_exact(credential.secret.len())
            .map_err(|_| AgentProviderCredentialError::Capacity)?;
        secret.extend_from_slice(&credential.secret);
        Ok(Self { secret })
    }

    fn sensitive_header(
        &self,
        provider: AgentProviderKind,
    ) -> Result<HeaderValue, AgentProviderCredentialError> {
        sensitive_header(provider, &self.secret)
    }
}

fn sensitive_header(
    provider: AgentProviderKind,
    secret: &[u8],
) -> Result<HeaderValue, AgentProviderCredentialError> {
    let prefix = if provider == AgentProviderKind::OpenAiResponses {
        b"Bearer ".as_slice()
    } else {
        &[]
    };
    let capacity = prefix
        .len()
        .checked_add(secret.len())
        .ok_or(AgentProviderCredentialError::Capacity)?;
    let mut encoded = Zeroizing::new(Vec::new());
    encoded
        .try_reserve_exact(capacity)
        .map_err(|_| AgentProviderCredentialError::Capacity)?;
    encoded.extend_from_slice(prefix);
    encoded.extend_from_slice(secret);
    // `HeaderValue::from_bytes` copies this temporary zeroizing buffer into
    // its library-owned `Bytes`. Construct it only at dispatch: the pinned
    // HTTP type has no zeroizing drop, so an earlier validation construction
    // would create needless secret residue as well as a second allocation.
    let mut value =
        HeaderValue::from_bytes(&encoded).map_err(|_| AgentProviderCredentialError::Content)?;
    value.set_sensitive(true);
    Ok(value)
}

/// Sticky cancellation shared from one run tree into its provider attempts.
#[derive(Clone)]
pub struct AgentProviderCancellation {
    inner: Arc<AgentProviderCancellationInner>,
}

struct AgentProviderCancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
    /// Linearizes the last pre-disclosure cancellation check with commit.
    /// This mutex never crosses an await or network operation.
    commit_gate: Mutex<()>,
}

impl AgentProviderCancellation {
    /// Creates one initially-live cancellation authority without a task or timer.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AgentProviderCancellationInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                commit_gate: Mutex::new(()),
            }),
        }
    }

    /// Makes cancellation sticky and wakes all currently admitted attempts.
    pub fn cancel(&self) -> bool {
        // A disclosure commit holds this gate only for its synchronous policy
        // mutation. Once this lock is acquired, either that commit already
        // linearized or this cancellation will be visible to its final check.
        let _commit_gate = self
            .inner
            .commit_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.publish_cancellation()
    }

    fn publish_cancellation(&self) -> bool {
        let first = !self.inner.cancelled.swap(true, Ordering::SeqCst);
        if first {
            self.inner.notify.notify_waiters();
        }
        first
    }

    /// Whether cancellation has become sticky.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Acquires the synchronous semantic-disclosure boundary. Poison means a
    /// prior commit panicked in an unwind-capable build; cancellation becomes
    /// sticky and no later disclosure may proceed through this authority.
    fn lock_commit_gate(&self) -> Option<MutexGuard<'_, ()>> {
        match self.inner.commit_gate.lock() {
            Ok(guard) => Some(guard),
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                self.publish_cancellation();
                drop(guard);
                None
            }
        }
    }

    fn shares_authority(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl Default for AgentProviderCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AgentProviderCancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Exact provider endpoint set.
///
/// Production construction remains private. Diagnostic loopback construction
/// is exported only by the release-forbidden `probe-harness` feature.
#[derive(Clone)]
pub struct ProviderEndpoints {
    openai: Url,
    openai_input_tokens: Url,
    anthropic: Url,
    https_only: bool,
}

impl ProviderEndpoints {
    fn production() -> Result<Self, AgentProviderTransportConfigError> {
        let openai = Url::parse(OPENAI_RESPONSES_URL)
            .map_err(|_| AgentProviderTransportConfigError::Endpoint)?;
        let openai_input_tokens = Url::parse(OPENAI_INPUT_TOKENS_URL)
            .map_err(|_| AgentProviderTransportConfigError::Endpoint)?;
        let anthropic = Url::parse(ANTHROPIC_MESSAGES_URL)
            .map_err(|_| AgentProviderTransportConfigError::Endpoint)?;
        if !exact_production_url(&openai, "api.openai.com", "/v1/responses")
            || !exact_production_url(
                &openai_input_tokens,
                "api.openai.com",
                "/v1/responses/input_tokens",
            )
            || !exact_production_url(&anthropic, "api.anthropic.com", "/v1/messages")
        {
            return Err(AgentProviderTransportConfigError::Endpoint);
        }
        Ok(Self {
            openai,
            openai_input_tokens,
            anthropic,
            https_only: true,
        })
    }

    fn endpoint(&self, endpoint: AgentProviderEndpoint) -> &Url {
        match endpoint {
            AgentProviderEndpoint::OpenAiResponses => &self.openai,
            AgentProviderEndpoint::AnthropicMessages => &self.anthropic,
        }
    }

    /// Builds diagnostic endpoints from exact IPv4 loopback URLs.
    ///
    /// The OpenAI input-count endpoint is derived from `openai`; callers can
    /// never supply a separate count URL.
    #[cfg(any(test, feature = "probe-harness"))]
    pub fn loopback(
        openai: &str,
        anthropic: &str,
    ) -> Result<Self, AgentProviderTransportConfigError> {
        let openai =
            parse_exact_loopback_url(openai).ok_or(AgentProviderTransportConfigError::Endpoint)?;
        let anthropic = parse_exact_loopback_url(anthropic)
            .ok_or(AgentProviderTransportConfigError::Endpoint)?;
        let mut openai_input_tokens = openai.clone();
        openai_input_tokens
            .path_segments_mut()
            .map_err(|_| AgentProviderTransportConfigError::Endpoint)?
            .pop_if_empty()
            .push("input_tokens");
        Ok(Self {
            openai_input_tokens,
            openai,
            anthropic,
            https_only: false,
        })
    }
}

fn exact_production_url(url: &Url, host: &str, path: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == path
        && url.query().is_none()
        && url.fragment().is_none()
}

/// Whether a diagnostic endpoint string is an exact explicit-port IPv4
/// loopback URL.
///
/// This helper is absent from shipping builds and does not authorize a
/// connection by itself. The entire source spelling must already equal the
/// canonical URL serialization, and its path must contain nonempty
/// ASCII-unreserved segments only. Percent escapes, dot segments, empty
/// segments, and trailing slashes are rejected before the source spelling can
/// be lost to URL normalization.
#[cfg(any(test, feature = "probe-harness"))]
pub fn exact_loopback_url(url: &str) -> bool {
    parse_exact_loopback_url(url).is_some()
}

#[cfg(any(test, feature = "probe-harness"))]
fn parse_exact_loopback_url(source: &str) -> Option<Url> {
    let url = Url::parse(source).ok()?;
    (url.as_str() == source
        && url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some_and(|port| port != 0)
        && url.username().is_empty()
        && url.password().is_none()
        && exact_loopback_path(url.path())
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

#[cfg(any(test, feature = "probe-harness"))]
fn exact_loopback_path(path: &str) -> bool {
    let Some(path) = path.strip_prefix('/') else {
        return false;
    };
    !path.is_empty()
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
                })
        })
}

struct TransportState {
    sealed: bool,
    active: Vec<TransportSlotKey>,
}

struct SharedTransportState {
    state: Mutex<TransportState>,
    shutdown: AgentProviderCancellation,
    drained: Notify,
    shutdown_waiting: AtomicBool,
}

struct AgentProviderTransportShutdownWaitGuard<'a> {
    waiting: &'a AtomicBool,
}

impl Drop for AgentProviderTransportShutdownWaitGuard<'_> {
    fn drop(&mut self) {
        self.waiting.store(false, Ordering::Release);
    }
}

/// Content-free snapshot of provider transport resource ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentProviderTransportSnapshot {
    sealed: bool,
    active: usize,
}

impl AgentProviderTransportSnapshot {
    /// Whether the transport refuses every future admission.
    pub const fn is_sealed(self) -> bool {
        self.sealed
    }

    /// Exact number of admitted attempts that have not returned a result.
    pub const fn active_attempts(self) -> usize {
        self.active
    }

    /// Process-local hard concurrency ceiling.
    pub const fn attempt_limit(self) -> usize {
        MAX_AGENT_PROVIDER_TRANSPORT_CALLS
    }

    /// Whether no provider attempt is currently admitted.
    ///
    /// Idle is observational only: new admission remains possible until the
    /// transport is sealed and therefore cannot prove shutdown quiescence.
    pub const fn is_idle(self) -> bool {
        self.active == 0
    }

    /// Whether sticky shutdown admission and every admitted attempt are drained.
    pub const fn is_quiescent(self) -> bool {
        self.sealed && self.active == 0
    }
}

/// Constructor-closed evidence of one permanently sealed, idle transport.
///
/// The proof is process-local and non-authorizing. It does not prove policy
/// settlement, provider usage accounting, durable audit delivery, or native
/// browser drain.
#[must_use]
pub struct AgentProviderTransportShutdownProof {
    snapshot: AgentProviderTransportSnapshot,
}

impl AgentProviderTransportShutdownProof {
    /// Exact sealed, zero-active snapshot admitted by the constructor.
    pub const fn snapshot(&self) -> AgentProviderTransportSnapshot {
        self.snapshot
    }
}

impl fmt::Debug for AgentProviderTransportShutdownProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderTransportShutdownProof")
            .field("sealed", &self.snapshot.is_sealed())
            .field("active_attempts", &self.snapshot.active_attempts())
            .finish()
    }
}

const _: () = assert!(
    std::mem::size_of::<AgentProviderTransportShutdownProof>()
        <= MAX_AGENT_PROVIDER_TRANSPORT_SHUTDOWN_PROOF_BYTES
);

/// Closed refusal to prove terminal provider-transport drain.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AgentProviderTransportShutdownError {
    /// Shared transport state could not be read safely.
    #[error("agent provider transport shutdown state is unavailable")]
    State,
    /// Provider admission has not been permanently sealed.
    #[error("agent provider transport is not sealed for shutdown")]
    Unsealed,
    /// At least one admitted provider attempt still owns its slot.
    #[error("agent provider transport shutdown still has active attempts")]
    Pending,
    /// The caller-owned absolute shutdown deadline elapsed before drain.
    #[error("agent provider transport shutdown deadline elapsed")]
    Deadline,
    /// Another caller already owns the sole provider-drain wait.
    #[error("agent provider transport shutdown wait is already active")]
    WaiterActive,
}

/// Failure to read or mutate transport resource state safely.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AgentProviderTransportStateError {
    /// A prior panic poisoned the bounded admission registry.
    #[error("agent provider transport state is unavailable")]
    Unavailable,
}

/// Shared fixed-endpoint provider transport with bounded synchronous admission.
///
/// Construction allocates a client and a four-entry registry. It starts no
/// task, timer, request, or socket. Clones share the same admission ceiling and
/// shutdown cancellation root.
#[derive(Clone)]
pub struct AgentProviderTransport {
    client: Client,
    config: AgentProviderTransportConfig,
    endpoints: ProviderEndpoints,
    shared: Arc<SharedTransportState>,
}

impl AgentProviderTransport {
    /// Builds the production client with two private exact HTTPS endpoints.
    pub fn try_new(
        config: AgentProviderTransportConfig,
    ) -> Result<Self, AgentProviderTransportConfigError> {
        Self::try_new_with_endpoints(config, ProviderEndpoints::production()?)
    }

    fn try_new_with_endpoints(
        config: AgentProviderTransportConfig,
        endpoints: ProviderEndpoints,
    ) -> Result<Self, AgentProviderTransportConfigError> {
        AgentProviderTransportConfig::try_new(
            config.request_timeout,
            config.connect_timeout,
            config.read_timeout,
        )?;
        let builder = Client::builder()
            .https_only(endpoints.https_only)
            .redirect(Policy::none())
            .referer(false)
            .retry(reqwest::retry::never())
            .timeout(config.request_timeout)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .http2_initial_stream_window_size(AGENT_PROVIDER_HTTP2_INITIAL_RECEIVE_WINDOW_BYTES)
            .http2_initial_connection_window_size(AGENT_PROVIDER_HTTP2_INITIAL_RECEIVE_WINDOW_BYTES)
            .http2_adaptive_window(false)
            .http2_max_frame_size(MAX_AGENT_PROVIDER_HTTP2_FRAME_BYTES)
            .http2_max_header_list_size(MAX_AGENT_PROVIDER_RESPONSE_HEADER_BYTES)
            .pool_max_idle_per_host(0)
            .tcp_nodelay(true)
            .user_agent(HeaderValue::from_static(PRODUCT_USER_AGENT));
        #[cfg(any(test, feature = "probe-harness"))]
        let builder = if endpoints.https_only {
            builder
        } else {
            builder.no_proxy()
        };
        let client = builder
            .build()
            .map_err(|_| AgentProviderTransportConfigError::Client)?;
        Ok(Self {
            client,
            config,
            endpoints,
            shared: Arc::new(SharedTransportState {
                state: Mutex::new(TransportState {
                    sealed: false,
                    active: Vec::with_capacity(MAX_AGENT_PROVIDER_TRANSPORT_CALLS),
                }),
                shutdown: AgentProviderCancellation::new(),
                drained: Notify::new(),
                shutdown_waiting: AtomicBool::new(false),
            }),
        })
    }

    /// Builds a redirect-free diagnostic client for exact loopback endpoints.
    ///
    /// This constructor is available only to tests and the release-forbidden
    /// `probe-harness` feature. The OpenAI count endpoint is always derived
    /// from `openai`, never accepted as independent caller input.
    #[cfg(any(test, feature = "probe-harness"))]
    pub fn try_new_loopback(
        config: AgentProviderTransportConfig,
        openai: &str,
        anthropic: &str,
    ) -> Result<Self, AgentProviderTransportConfigError> {
        Self::try_new_with_endpoints(config, ProviderEndpoints::loopback(openai, anthropic)?)
    }

    /// Validated transport deadlines.
    pub const fn config(&self) -> AgentProviderTransportConfig {
        self.config
    }

    /// Returns the current bounded resource snapshot.
    pub fn snapshot(
        &self,
    ) -> Result<AgentProviderTransportSnapshot, AgentProviderTransportStateError> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| AgentProviderTransportStateError::Unavailable)?;
        Ok(AgentProviderTransportSnapshot {
            sealed: state.sealed,
            active: state.active.len(),
        })
    }

    /// Proves that sticky shutdown admission and all provider attempts drained.
    ///
    /// Call [`Self::seal`] first, cancel or terminally settle every retained
    /// attempt, and retry this nonblocking check under the process shutdown
    /// deadline. Once returned, the fact remains true because transport
    /// admission cannot reopen after sealing.
    pub fn try_prove_shutdown(
        &self,
    ) -> Result<AgentProviderTransportShutdownProof, AgentProviderTransportShutdownError> {
        let snapshot = self
            .snapshot()
            .map_err(|_| AgentProviderTransportShutdownError::State)?;
        if !snapshot.is_sealed() {
            return Err(AgentProviderTransportShutdownError::Unsealed);
        }
        if !snapshot.is_idle() {
            return Err(AgentProviderTransportShutdownError::Pending);
        }
        Ok(AgentProviderTransportShutdownProof { snapshot })
    }

    /// Seals admission and asynchronously waits for every provider slot to drain.
    ///
    /// Sealing is synchronous, sticky, and happens before the deadline check,
    /// so dropping or timing out this future cannot reopen provider work. The
    /// returned future registers before sampling state, sleeps only on a slot-
    /// release notification or the caller's absolute monotonic deadline, and
    /// creates no worker, polling timer, socket, or provider request. At most
    /// one drain future may exist for the shared transport; dropping it
    /// synchronously releases only that wait admission, never the sticky seal.
    ///
    /// A successful return proves only transport-slot drain. The caller must
    /// independently retain and settle every terminal provider result's policy
    /// and usage authority.
    pub fn seal_and_prove_shutdown_until(
        &self,
        deadline: Instant,
    ) -> Result<
        impl std::future::Future<
                Output = Result<
                    AgentProviderTransportShutdownProof,
                    AgentProviderTransportShutdownError,
                >,
            > + Send
            + '_,
        AgentProviderTransportShutdownError,
    > {
        self.seal();
        self.shared
            .shutdown_waiting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| AgentProviderTransportShutdownError::WaiterActive)?;
        let wait_guard = AgentProviderTransportShutdownWaitGuard {
            waiting: &self.shared.shutdown_waiting,
        };
        Ok(async move {
            let _wait_guard = wait_guard;
            let deadline_sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
            tokio::pin!(deadline_sleep);

            loop {
                if Instant::now() >= deadline {
                    return Err(AgentProviderTransportShutdownError::Deadline);
                }

                let drained = self.shared.drained.notified();
                tokio::pin!(drained);
                drained.as_mut().enable();
                match self.try_prove_shutdown() {
                    Ok(proof) => return Ok(proof),
                    Err(AgentProviderTransportShutdownError::Pending) => {}
                    Err(error) => return Err(error),
                }

                tokio::select! {
                    () = &mut drained => {}
                    () = &mut deadline_sleep => {
                        return Err(AgentProviderTransportShutdownError::Deadline);
                    }
                }
            }
        })
    }

    /// Permanently refuses new calls and cancels every admitted attempt.
    pub fn seal(&self) {
        // Publish sticky cancellation before the admission flag so no caller
        // can observe shutdown, race through the separate cancellation check,
        // and commit after seal's linearization point.
        self.shared.shutdown.cancel();
        match self.shared.state.lock() {
            Ok(mut state) => state.sealed = true,
            Err(poisoned) => poisoned.into_inner().sealed = true,
        }
    }

    /// Admits, commits, and binds one exact request without starting network I/O.
    ///
    /// All credential, endpoint, cancellation, duplicate, capacity, and
    /// shutdown checks finish before semantic disclosure commits. Once this
    /// returns an attempt, every later outcome is post-commit and must consume
    /// the returned active policy authority exactly once.
    pub fn try_admit(
        &self,
        input: AgentProviderTransportInput,
        policy: &mut AgentRunPolicy,
        credential: &AgentProviderCredential,
        cancellation: AgentProviderCancellation,
    ) -> Result<AgentProviderAttempt, AgentProviderAdmissionError> {
        let provider = input.request().config().provider();
        let endpoint = input.request().endpoint();
        let Some(deadline) = Instant::now().checked_add(self.config.request_timeout) else {
            let _outcome = input
                .refuse(policy)
                .map_err(AgentProviderAdmissionError::Settlement)?;
            return Err(AgentProviderAdmissionError::State);
        };
        if credential.provider() != provider {
            let _outcome = input
                .refuse(policy)
                .map_err(AgentProviderAdmissionError::Settlement)?;
            return Err(AgentProviderAdmissionError::CredentialProvider);
        }
        // A chosen model's request is never sent to a fixed endpoint.
        if !provider_endpoint_matches(provider, endpoint)
            || input.request().config().response_route()
                == crate::AgentProviderResponseRoute::ModelReported
        {
            let _outcome = input
                .refuse(policy)
                .map_err(AgentProviderAdmissionError::Settlement)?;
            return Err(AgentProviderAdmissionError::Endpoint);
        }
        let credential = match AgentProviderAttemptCredential::try_from_credential(credential) {
            Ok(credential) => credential,
            Err(_) => {
                let _outcome = input
                    .refuse(policy)
                    .map_err(AgentProviderAdmissionError::Settlement)?;
                return Err(AgentProviderAdmissionError::Credential);
            }
        };
        self.commit_admitted(
            input,
            policy,
            cancellation,
            deadline,
            Some(credential),
            None,
        )
    }

    /// Admits one turn on the person's chosen model without starting a call.
    ///
    /// The request must carry the provider-reported accounting mode bound to
    /// exactly this model's pricing schedule. Every capacity, duplicate,
    /// cancellation and shutdown check, and the disclosure commit, are the
    /// fixed-endpoint path's own.
    pub fn try_admit_model(
        &self,
        input: AgentProviderTransportInput,
        policy: &mut AgentRunPolicy,
        model: &model_client::AgentProviderModelClient,
        cancellation: AgentProviderCancellation,
    ) -> Result<AgentProviderAttempt, AgentProviderAdmissionError> {
        let Some(deadline) = Instant::now().checked_add(self.config.request_timeout) else {
            let _outcome = input
                .refuse(policy)
                .map_err(AgentProviderAdmissionError::Settlement)?;
            return Err(AgentProviderAdmissionError::State);
        };
        let config = input.request().config();
        if input.request().endpoint() != AgentProviderEndpoint::OpenAiResponses
            || config.input_accounting_mode()
                != crate::AgentProviderInputAccountingMode::ProviderReportedAfterConservativeReservation
            || !model.binds(config)
        {
            let _outcome = input
                .refuse(policy)
                .map_err(AgentProviderAdmissionError::Settlement)?;
            return Err(AgentProviderAdmissionError::ModelBinding);
        }
        self.commit_admitted(
            input,
            policy,
            cancellation,
            deadline,
            None,
            Some(model.clone()),
        )
    }

    fn commit_admitted(
        &self,
        input: AgentProviderTransportInput,
        policy: &mut AgentRunPolicy,
        cancellation: AgentProviderCancellation,
        deadline: Instant,
        credential: Option<AgentProviderAttemptCredential>,
        model: Option<model_client::AgentProviderModelClient>,
    ) -> Result<AgentProviderAttempt, AgentProviderAdmissionError> {
        let has_images = lead::image_fence::provider_body_has_image(input.request().body())
            || policy.has_model_image_history();
        // An optional caller fence can only restrict ordinary requests. Every
        // image (including retained replay and exact counting) requires the
        // original policy's exact native+consent fence; it cannot be replaced.
        let optional = input.image_fence();
        let image_fence = if has_images {
            policy.model_image_fence().filter(|required| {
                optional
                    .as_ref()
                    .is_none_or(|extra| std::sync::Arc::ptr_eq(required, extra))
            })
        } else {
            optional
        };
        if (has_images && image_fence.is_none())
            || image_fence
                .as_ref()
                .is_some_and(|fence| !fence.while_current(&mut || {}))
        {
            let _ = input
                .cancel(policy)
                .map_err(AgentProviderAdmissionError::Settlement)?;
            return Err(AgentProviderAdmissionError::Cancelled);
        }
        let call = input.request().call();
        let provider = input.request().config().provider();
        let endpoint = input.request().endpoint();
        let mut slot = match self.reserve(call) {
            Ok(slot) => slot,
            Err(error) => {
                let _outcome = input
                    .refuse(policy)
                    .map_err(AgentProviderAdmissionError::Settlement)?;
                return Err(error.into());
            }
        };
        let cancellation_gate = match cancellation.lock_commit_gate() {
            Some(gate) => gate,
            None => {
                let _outcome = input
                    .cancel(policy)
                    .map_err(AgentProviderAdmissionError::Settlement)?;
                drop(slot);
                return Err(AgentProviderAdmissionError::Cancelled);
            }
        };
        // The transport shutdown authority is private, but tolerate an exact
        // shared authority defensively so a future internal composition cannot
        // self-deadlock while acquiring the two commit gates.
        let shutdown_gate = if cancellation.shares_authority(&self.shared.shutdown) {
            None
        } else {
            match self.shared.shutdown.lock_commit_gate() {
                Some(gate) => Some(gate),
                None => {
                    drop(cancellation_gate);
                    self.seal();
                    let _outcome = input
                        .cancel(policy)
                        .map_err(AgentProviderAdmissionError::Settlement)?;
                    drop(slot);
                    return Err(AgentProviderAdmissionError::Cancelled);
                }
            }
        };
        if cancellation.is_cancelled() || self.shared.shutdown.is_cancelled() {
            drop(shutdown_gate);
            drop(cancellation_gate);
            let _outcome = input
                .cancel(policy)
                .map_err(AgentProviderAdmissionError::Settlement)?;
            drop(slot);
            return Err(AgentProviderAdmissionError::Cancelled);
        }
        let committed = if let Some(fence) = &image_fence {
            let mut owned = Some(input);
            let mut committed = None;
            let current = fence.while_current(&mut || {
                if let Some(input) = owned.take() {
                    committed = Some(input.commit(policy));
                }
            });
            if !current || committed.is_none() {
                if let Some(input) = owned.take() {
                    let _ = input
                        .cancel(policy)
                        .map_err(AgentProviderAdmissionError::Settlement)?;
                }
                return Err(AgentProviderAdmissionError::Cancelled);
            }
            committed.ok_or(AgentProviderAdmissionError::Cancelled)?
        } else {
            input.commit(policy)
        };
        let committed = match committed {
            Ok(committed) => committed,
            Err(error) => {
                drop(shutdown_gate);
                drop(cancellation_gate);
                drop(slot);
                return Err(AgentProviderAdmissionError::Settlement(error));
            }
        };
        slot.mark_committed();
        drop(shutdown_gate);
        drop(cancellation_gate);
        Ok(AgentProviderAttempt {
            image_fence,
            client: self.client.clone(),
            endpoint: self.endpoints.endpoint(endpoint).clone(),
            input_token_endpoint: (provider == AgentProviderKind::OpenAiResponses)
                .then(|| self.endpoints.openai_input_tokens.clone()),
            provider,
            prior_disclosure_stage: AgentProviderDisclosureStage::NotDispatched,
            credential,
            model,
            model_reply: None,
            core: Some(AgentProviderAttemptCore::PreDispatch {
                committed: Box::new(committed),
                slot: Some(slot),
            }),
            phase: AgentProviderAttemptPhase::Ready,
            cancellation,
            shutdown: self.shared.shutdown.clone(),
            deadline,
        })
    }

    fn reserve(&self, call: AgentProviderCallIdentity) -> Result<AgentProviderSlot, ReserveError> {
        self.reserve_key(TransportSlotKey::Browser(call))
    }
    fn reserve_key(&self, call: TransportSlotKey) -> Result<AgentProviderSlot, ReserveError> {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                poisoned.into_inner().sealed = true;
                self.shared.shutdown.cancel();
                return Err(ReserveError::State);
            }
        };
        if state.sealed {
            return Err(ReserveError::Sealed);
        }
        if state.active.contains(&call) {
            return Err(ReserveError::Duplicate);
        }
        if state.active.len() >= MAX_AGENT_PROVIDER_TRANSPORT_CALLS {
            return Err(ReserveError::Capacity);
        }
        state.active.push(call);
        Ok(AgentProviderSlot {
            shared: Arc::clone(&self.shared),
            call,
            committed: false,
            completed: false,
        })
    }
}

impl fmt::Debug for AgentProviderTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderTransport")
            .field("config", &self.config)
            .field("snapshot", &self.snapshot().ok())
            .field("endpoints", &"[fixed]")
            .finish()
    }
}

fn provider_endpoint_matches(provider: AgentProviderKind, endpoint: AgentProviderEndpoint) -> bool {
    matches!(
        (provider, endpoint),
        (
            AgentProviderKind::OpenAiResponses,
            AgentProviderEndpoint::OpenAiResponses
        ) | (
            AgentProviderKind::AnthropicMessages,
            AgentProviderEndpoint::AnthropicMessages
        )
    )
}

enum ReserveError {
    Sealed,
    Capacity,
    Duplicate,
    State,
}

impl From<ReserveError> for AgentProviderAdmissionError {
    fn from(error: ReserveError) -> Self {
        match error {
            ReserveError::Sealed => Self::Sealed,
            ReserveError::Capacity => Self::Capacity,
            ReserveError::Duplicate => Self::Duplicate,
            ReserveError::State => Self::State,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TransportSlotKey {
    Browser(AgentProviderCallIdentity),
    Planning(ulid::Ulid),
}

struct AgentProviderSlot {
    shared: Arc<SharedTransportState>,
    call: TransportSlotKey,
    committed: bool,
    completed: bool,
}

impl AgentProviderSlot {
    fn mark_committed(&mut self) {
        self.committed = true;
    }

    fn mark_completed(&mut self) {
        self.completed = true;
    }

    fn fail_stop(&self) {
        // Match the public shutdown order: make cancellation sticky before
        // publishing the admission seal, then recover poison only to retain
        // that terminal state. No transport clone may admit after this call.
        self.shared.shutdown.cancel();
        match self.shared.state.lock() {
            Ok(mut state) => state.sealed = true,
            Err(poisoned) => poisoned.into_inner().sealed = true,
        }
    }
}

impl Drop for AgentProviderSlot {
    fn drop(&mut self) {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                self.shared.shutdown.cancel();
                let mut state = poisoned.into_inner();
                state.sealed = true;
                state
            }
        };
        if self.committed && !self.completed {
            state.sealed = true;
            self.shared.shutdown.cancel();
        }
        if let Some(index) = state.active.iter().position(|call| call == &self.call) {
            state.active.swap_remove(index);
        } else {
            state.sealed = true;
            self.shared.shutdown.cancel();
        }
        let idle = state.active.is_empty();
        drop(state);
        if idle {
            self.shared.drained.notify_waiters();
        }
    }
}

/// Pre-network refusal from exact provider transport admission.
#[derive(Debug, Error)]
pub enum AgentProviderAdmissionError {
    /// Credential was bound to the other provider protocol.
    #[error("agent provider credential does not match the request provider")]
    CredentialProvider,
    /// Credential could not be encoded as the provider's sensitive header.
    #[error("agent provider credential header is invalid")]
    Credential,
    /// Provider kind and fixed request endpoint did not agree.
    #[error("agent provider request endpoint is invalid")]
    Endpoint,
    /// Four other model calls currently own all transport capacity.
    #[error("agent provider transport concurrency ceiling reached")]
    Capacity,
    /// The same exact call already owns a transport attempt.
    #[error("agent provider transport call is already active")]
    Duplicate,
    /// Transport shutdown permanently closed admission.
    #[error("agent provider transport is sealed")]
    Sealed,
    /// Exact cancellation won before semantic disclosure committed.
    #[error("agent provider request was cancelled before commitment")]
    Cancelled,
    /// The request was not bound to the chosen model it was admitted for.
    #[error("agent provider request is not bound to the chosen model")]
    ModelBinding,
    /// Bounded transport admission state became unavailable.
    #[error("agent provider transport state is unavailable")]
    State,
    /// One-shot policy or semantic input settlement failed.
    #[error("agent provider input settlement failed")]
    Settlement(#[source] AgentProviderRequestError),
}

/// Consumer decision after receiving one normalized nonempty stream batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProviderBatchDisposition {
    /// Continue reading the exact provider stream.
    Continue,
    /// Stop this committed call and settle it as cancelled.
    Cancel,
}

/// Borrowed terminal network/decoder outcome for one committed request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProviderTransportOutcome<'a> {
    /// The provider decoder reached one complete, framed response-body EOF.
    Stream(&'a AgentProviderStreamConclusion),
    /// HTTP, transport, cancellation, or protocol failed outside an EOF terminal.
    Failed(AgentProviderFailure),
}

enum AgentProviderTransportOutcomeOwned {
    Stream(Box<AgentProviderFinishedStream>),
    Failed(AgentProviderFailure),
}

/// Trustworthy provider-usage knowledge retained at the transport boundary.
///
/// This value concerns model-generation usage only. External disclosure is
/// tracked independently by [`AgentProviderDisclosureStage`], because an
/// authenticated input-token count can disclose the canonical input projection
/// while model usage remains exactly zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProviderUsageKnowledge {
    /// The model-generation send was never polled, so model usage and cost are zero.
    ExactZeroBeforeModelDispatch,
    /// The fixed provider decoder supplied internally consistent token counters.
    ProviderReported(AgentProviderUsage),
    /// Network dispatch may have occurred and no trustworthy usage was returned.
    UnknownAfterDispatch,
}

/// Furthest provider disclosure reached by one terminal attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProviderDisclosureStage {
    /// No provider HTTP request was polled.
    NotDispatched,
    /// The canonical token-relevant projection reached OpenAI counting, but
    /// the model-generation endpoint was provably not polled.
    InputTokenCountDisclosed,
    /// The model-generation request may have reached the provider.
    ModelRequestMayHaveDispatched,
}

/// Terminal result joined to exact policy authority and pricing identity.
#[must_use]
pub struct AgentProviderTransportResult {
    active: AgentActiveModelCall,
    config: AgentProviderCallConfig,
    input_metric_receipt: AgentProviderInputMetricReceipt,
    outcome: AgentProviderTransportOutcomeOwned,
    disclosure_stage: AgentProviderDisclosureStage,
    continuation: Option<Box<AgentProviderContinuationSeed>>,
    model_reply: Option<crate::AgentProviderModelReplyFacts>,
}

impl AgentProviderTransportResult {
    /// Closed facts about a chosen model's reply, when one was decoded.
    pub const fn model_reply(&self) -> Option<crate::AgentProviderModelReplyFacts> {
        self.model_reply
    }

    /// Exact non-authorizing correlation for this result.
    pub fn call(&self) -> AgentProviderCallIdentity {
        AgentProviderCallIdentity::from_active(&self.active)
    }

    /// Exact policy authority that must receive one terminal settlement.
    pub const fn active(&self) -> &AgentActiveModelCall {
        &self.active
    }

    /// Exact provider, model, tokenizer, pricing, and response bounds for the call.
    pub const fn config(&self) -> &AgentProviderCallConfig {
        &self.config
    }

    /// Final content-free metrics for the input that crossed disclosure commit.
    ///
    /// Provider-exact success carries the authenticated `ProviderExact` whole-
    /// request count. A count-phase terminal carries the original conservative
    /// reservation, now final because the attempt can no longer reach Counted.
    pub const fn input_metric_receipt(&self) -> AgentProviderInputMetricReceipt {
        self.input_metric_receipt
    }

    /// Closed content-free terminal transport outcome.
    pub fn outcome(&self) -> AgentProviderTransportOutcome<'_> {
        match &self.outcome {
            AgentProviderTransportOutcomeOwned::Stream(finished) => {
                AgentProviderTransportOutcome::Stream(finished.conclusion_ref())
            }
            AgentProviderTransportOutcomeOwned::Failed(failure) => {
                AgentProviderTransportOutcome::Failed(*failure)
            }
        }
    }

    /// Exact usage evidence that determines the only safe settlement route.
    pub fn usage_knowledge(&self) -> AgentProviderUsageKnowledge {
        match &self.outcome {
            AgentProviderTransportOutcomeOwned::Stream(finished) => match finished.conclusion() {
                AgentProviderStreamConclusion::Completed(completion) => {
                    AgentProviderUsageKnowledge::ProviderReported(completion.usage())
                }
                AgentProviderStreamConclusion::Failed(failure) => match failure.usage() {
                    Some(usage) => AgentProviderUsageKnowledge::ProviderReported(usage),
                    None => AgentProviderUsageKnowledge::UnknownAfterDispatch,
                },
            },
            AgentProviderTransportOutcomeOwned::Failed(_) => match self.disclosure_stage {
                AgentProviderDisclosureStage::NotDispatched
                | AgentProviderDisclosureStage::InputTokenCountDisclosed => {
                    AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
                }
                AgentProviderDisclosureStage::ModelRequestMayHaveDispatched => {
                    AgentProviderUsageKnowledge::UnknownAfterDispatch
                }
            },
        }
    }

    /// Furthest external disclosure reached independently from model usage.
    pub const fn disclosure_stage(&self) -> AgentProviderDisclosureStage {
        self.disclosure_stage
    }

    /// Converts transport evidence into the sole safe policy-settlement route.
    ///
    /// Provider-reported usage retains the exact fixed provider/model/tokenizer
    /// configuration until trusted pricing supplies cost. Failures before model
    /// dispatch settle exact-zero model usage even when counting disclosed input;
    /// ambiguous model sends consume the complete reservation.
    pub fn into_policy_settlement(self) -> AgentProviderPolicySettlement {
        let Self {
            active,
            config,
            input_metric_receipt: _,
            outcome,
            disclosure_stage,
            continuation,
            model_reply: _,
        } = self;
        match outcome {
            AgentProviderTransportOutcomeOwned::Stream(finished) => {
                let conclusion = finished.conclusion();
                if finished.usage().is_some() {
                    match AgentProviderPricingSettlement::try_from_provider_eof(
                        active,
                        config,
                        *finished,
                        continuation.map(|seed| *seed),
                    ) {
                        Ok(settlement) => {
                            AgentProviderPolicySettlement::PricingRequired(Box::new(settlement))
                        }
                        Err(error) => AgentProviderPolicySettlement::Immediate(
                            AgentProviderImmediateSettlement {
                                active: error.into_active(),
                                outcome: AgentProviderTransportOutcomeOwned::Failed(
                                    protocol_failure_value(),
                                ),
                                accounting: AgentProviderImmediateAccounting::ReservationCeiling(
                                    AgentModelCallUnaccountedSettlement::ProviderFailed,
                                ),
                            },
                        ),
                    }
                } else {
                    let settlement = match conclusion {
                        AgentProviderStreamConclusion::Failed(failure) => {
                            unaccounted_settlement_for_failure(failure.failure())
                        }
                        AgentProviderStreamConclusion::Completed(_) => {
                            AgentModelCallUnaccountedSettlement::ProviderFailed
                        }
                    };
                    AgentProviderPolicySettlement::Immediate(AgentProviderImmediateSettlement {
                        active,
                        outcome: AgentProviderTransportOutcomeOwned::Stream(finished),
                        accounting: AgentProviderImmediateAccounting::ReservationCeiling(
                            settlement,
                        ),
                    })
                }
            }
            AgentProviderTransportOutcomeOwned::Failed(failure) => {
                let settlement = settlement_for_failure(failure);
                let accounting = match disclosure_stage {
                    AgentProviderDisclosureStage::NotDispatched
                    | AgentProviderDisclosureStage::InputTokenCountDisclosed => {
                        AgentProviderImmediateAccounting::ExactZero(settlement)
                    }
                    AgentProviderDisclosureStage::ModelRequestMayHaveDispatched => {
                        AgentProviderImmediateAccounting::ReservationCeiling(
                            unaccounted_settlement_for_failure(failure),
                        )
                    }
                };
                AgentProviderPolicySettlement::Immediate(AgentProviderImmediateSettlement {
                    active,
                    outcome: AgentProviderTransportOutcomeOwned::Failed(failure),
                    accounting,
                })
            }
        }
    }
}

impl fmt::Debug for AgentProviderTransportResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderTransportResult")
            .field("active", &self.active)
            .field("config", &self.config)
            .field("input_metric_receipt", &self.input_metric_receipt)
            .field("outcome", &self.outcome())
            .field("usage", &self.usage_knowledge())
            .field("disclosure_stage", &self.disclosure_stage)
            .field("continuation", &self.continuation.is_some())
            .finish()
    }
}

/// Move-only safe settlement selected from terminal transport evidence.
#[must_use]
pub enum AgentProviderPolicySettlement {
    /// No pricing lookup is needed: usage is exact zero or unknowable.
    Immediate(AgentProviderImmediateSettlement),
    /// Exact normalized tokens require the matching trusted pricing revision.
    PricingRequired(Box<AgentProviderPricingSettlement>),
}

impl AgentProviderPolicySettlement {
    /// Exact non-authorizing call correlation retained with policy authority.
    pub fn call(&self) -> AgentProviderCallIdentity {
        match self {
            Self::Immediate(settlement) => settlement.call(),
            Self::PricingRequired(settlement) => settlement.call(),
        }
    }
}

impl fmt::Debug for AgentProviderPolicySettlement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Immediate(settlement) => formatter
                .debug_tuple("Immediate")
                .field(settlement)
                .finish(),
            Self::PricingRequired(settlement) => formatter
                .debug_tuple("PricingRequired")
                .field(settlement)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AgentProviderImmediateAccounting {
    ExactZero(AgentModelCallSettlement),
    ReservationCeiling(AgentModelCallUnaccountedSettlement),
}

/// Move-only terminal authority that can settle without a pricing lookup.
#[must_use]
pub struct AgentProviderImmediateSettlement {
    active: AgentActiveModelCall,
    outcome: AgentProviderTransportOutcomeOwned,
    accounting: AgentProviderImmediateAccounting,
}

impl AgentProviderImmediateSettlement {
    /// Exact non-authorizing call correlation.
    pub fn call(&self) -> AgentProviderCallIdentity {
        AgentProviderCallIdentity::from_active(&self.active)
    }

    /// Content-free terminal transport outcome retained for supervision.
    pub fn outcome(&self) -> AgentProviderTransportOutcome<'_> {
        match &self.outcome {
            AgentProviderTransportOutcomeOwned::Stream(finished) => {
                AgentProviderTransportOutcome::Stream(finished.conclusion_ref())
            }
            AgentProviderTransportOutcomeOwned::Failed(failure) => {
                AgentProviderTransportOutcome::Failed(*failure)
            }
        }
    }

    /// Completed, provider-failed, or cancelled policy terminal class.
    pub const fn settlement(&self) -> AgentModelCallSettlement {
        match self.accounting {
            AgentProviderImmediateAccounting::ExactZero(settlement) => settlement,
            AgentProviderImmediateAccounting::ReservationCeiling(settlement) => match settlement {
                AgentModelCallUnaccountedSettlement::ProviderFailed => {
                    AgentModelCallSettlement::ProviderFailed
                }
                AgentModelCallUnaccountedSettlement::Cancelled => {
                    AgentModelCallSettlement::Cancelled
                }
            },
        }
    }

    /// Exact-zero or reservation-ceiling accounting selected by transport proof.
    pub const fn usage_accounting(&self) -> AgentModelUsageAccounting {
        match self.accounting {
            AgentProviderImmediateAccounting::ExactZero(_) => AgentModelUsageAccounting::Exact,
            AgentProviderImmediateAccounting::ReservationCeiling(_) => {
                AgentModelUsageAccounting::ReservationCeiling
            }
        }
    }

    /// Consumes the exact active authority in the selected safe policy path.
    ///
    /// A wrong or sealed policy is rejected before this owner is destructured,
    /// returning the complete settlement through the recoverable error.
    pub fn settle(
        self,
        policy: &mut AgentRunPolicy,
    ) -> Result<AgentModelCallReceipt, AgentProviderImmediateSettlementError> {
        if let Err(error) = policy.prevalidate_active_model_call(&self.active) {
            return Err(AgentProviderImmediateSettlementError::PolicyPrecondition {
                error,
                unsettled: Box::new(self),
            });
        }
        let Self {
            active, accounting, ..
        } = self;
        match accounting {
            AgentProviderImmediateAccounting::ExactZero(settlement) => {
                policy.settle_model_call(active, settlement, 0, 0, 0)
            }
            AgentProviderImmediateAccounting::ReservationCeiling(settlement) => {
                policy.settle_model_call_unaccounted(active, settlement)
            }
        }
        .map_err(AgentProviderImmediateSettlementError::Policy)
    }
}

impl fmt::Debug for AgentProviderImmediateSettlement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderImmediateSettlement")
            .field("active", &self.active)
            .field("outcome", &self.outcome())
            .field("settlement", &self.settlement())
            .field("usage_accounting", &self.usage_accounting())
            .finish()
    }
}

/// Failure while settling terminal transport evidence without a pricing lookup.
///
/// A policy precondition refusal retains the complete move-only terminal owner,
/// because policy state was not changed. A post-prevalidation policy failure has
/// consumed the terminal transition and is therefore fail-stopped.
#[derive(Debug, Error)]
pub enum AgentProviderImmediateSettlementError {
    /// The supplied policy was sealed or did not own this exact active call.
    #[error("agent provider immediate settlement policy precondition failed")]
    PolicyPrecondition {
        /// Content-free policy refusal.
        error: AgentPolicyError,
        /// Complete authority retained for the exact live policy.
        unsettled: Box<AgentProviderImmediateSettlement>,
    },
    /// Policy consumed the terminal transition and failed stopped.
    #[error("agent provider immediate policy settlement failed")]
    Policy(#[source] AgentPolicyError),
}

impl AgentProviderImmediateSettlementError {
    /// Content-free policy refusal when the settlement owner was retained.
    pub const fn policy_precondition_error(&self) -> Option<AgentPolicyError> {
        match self {
            Self::PolicyPrecondition { error, .. } => Some(*error),
            Self::Policy(_) => None,
        }
    }

    /// Recovers exact authority only when policy refused before policy use.
    pub fn into_unsettled(self) -> Option<AgentProviderImmediateSettlement> {
        match self {
            Self::PolicyPrecondition { unsettled, .. } => Some(*unsettled),
            Self::Policy(_) => None,
        }
    }
}

/// One admitted, abortable provider operation.
///
/// The operation owns every terminal policy and slot authority. Its count and
/// generation drives borrow this value instead of consuming it, so dropping a
/// drive future at a stronger host deadline leaves [`Self::abort`] able to
/// return the exact terminal settlement owner. Ordinary cancellation should
/// still let the borrowed drive finish and return its normal terminal.
#[must_use]
pub struct AgentProviderAttempt {
    image_fence: Option<Arc<dyn zephium_core::work::model::WorkModelImageFence>>,
    client: Client,
    endpoint: Url,
    input_token_endpoint: Option<Url>,
    provider: AgentProviderKind,
    prior_disclosure_stage: AgentProviderDisclosureStage,
    credential: Option<AgentProviderAttemptCredential>,
    model: Option<model_client::AgentProviderModelClient>,
    model_reply: Option<crate::AgentProviderModelReplyFacts>,
    core: Option<AgentProviderAttemptCore>,
    phase: AgentProviderAttemptPhase,
    cancellation: AgentProviderCancellation,
    shutdown: AgentProviderCancellation,
    deadline: Instant,
}

/// Terminal authority that never enters a droppable network-drive future.
struct AgentProviderTerminalCore {
    input: AgentCommittedProviderInput,
    config: AgentProviderCallConfig,
    continuation: Option<AgentProviderContinuationSeed>,
    slot: Option<AgentProviderSlot>,
}

/// One phase of an admitted abortable operation.
enum AgentProviderAttemptCore {
    /// Counting has not yet moved the request body into a generation drive.
    PreDispatch {
        committed: Box<AgentCommittedProviderRequest>,
        slot: Option<AgentProviderSlot>,
    },
    /// A generation drive owns only request/body/decoder temporaries.
    Generation(Box<AgentProviderTerminalCore>),
}

/// Exclusive public phase of one abortable operation owner.
enum AgentProviderAttemptPhase {
    Ready,
    CountDriving,
    Counted(AgentProviderExactInputCount),
    GenerationDriving,
    Terminal,
}

/// Content-free reason the controller had to abort a borrowed provider drive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProviderAbortReason {
    /// The runtime's absolute cooperative-drain deadline elapsed.
    HostDeadline,
    /// The trusted controller detected an unrecoverable integration fault.
    ControllerFault,
}

/// Content-free refusal from an already-terminal or interrupted operation.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AgentProviderAttemptStateError {
    /// A prior drive or abort already consumed the terminal authority.
    #[error("agent provider operation is already terminal")]
    Terminal,
    /// A dropped count drive may have disclosed input and must be aborted.
    #[error("agent provider count drive requires terminal recovery")]
    CountRecoveryRequired,
    /// A dropped generation drive may have dispatched the model and must abort.
    #[error("agent provider generation drive requires terminal recovery")]
    GenerationRecoveryRequired,
}

/// Result of the authenticated OpenAI input-token phase.
#[must_use]
pub enum AgentProviderExactCountOutcome<'operation> {
    /// Exact accounting was bound and the model request remains undispatched.
    Counted(AgentProviderCountedAttempt<'operation>),
    /// Counting or its gates failed; terminal policy authority is retained.
    Failed(AgentProviderTransportResult),
    /// No live count drive could be started without risking duplicate disclosure.
    Unavailable(AgentProviderAttemptStateError),
}

impl fmt::Debug for AgentProviderExactCountOutcome<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Counted(counted) => formatter.debug_tuple("Counted").field(counted).finish(),
            Self::Failed(result) => formatter.debug_tuple("Failed").field(result).finish(),
            Self::Unavailable(error) => formatter.debug_tuple("Unavailable").field(error).finish(),
        }
    }
}

/// Borrowed OpenAI provider-exact operation after authenticated counting.
#[must_use]
pub struct AgentProviderCountedAttempt<'operation> {
    operation: &'operation mut AgentProviderAttempt,
    count: AgentProviderExactInputCount,
    input_metrics: AgentProviderInputMetrics,
    input_metric_receipt: AgentProviderInputMetricReceipt,
}

impl AgentProviderCountedAttempt<'_> {
    /// Exact authenticated count bound to this attempt's immutable request.
    pub fn count(&self) -> &AgentProviderExactInputCount {
        &self.count
    }

    /// Updated input metrics carrying `ProviderExact` structured accounting.
    pub fn input_metrics(&self) -> AgentProviderInputMetrics {
        self.input_metrics
    }

    /// Final ProviderExact input metrics bound before generation dispatch.
    pub fn input_metric_receipt(&self) -> AgentProviderInputMetricReceipt {
        self.input_metric_receipt
    }

    /// Cancels before generation while preserving count-disclosure evidence.
    pub fn cancel_without_model_dispatch(
        self,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError> {
        self.operation.cancel_without_dispatch()
    }

    /// Dispatches the single model request after exact input accounting.
    pub async fn execute<F>(
        self,
        consume: F,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError>
    where
        F: FnMut(AgentProviderStreamBatch) -> AgentProviderBatchDisposition,
    {
        self.operation.execute_model(consume).await
    }
}

impl fmt::Debug for AgentProviderCountedAttempt<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderCountedAttempt")
            .field("operation", &self.operation)
            .field("count", &self.count)
            .finish()
    }
}

impl AgentProviderAttempt {
    fn pre_dispatch(
        &self,
    ) -> Result<&AgentCommittedProviderRequest, AgentProviderAttemptStateError> {
        match (&self.phase, self.core.as_ref()) {
            (
                AgentProviderAttemptPhase::Ready,
                Some(AgentProviderAttemptCore::PreDispatch { committed, .. }),
            )
            | (
                AgentProviderAttemptPhase::Counted(_),
                Some(AgentProviderAttemptCore::PreDispatch { committed, .. }),
            ) => Ok(committed),
            (AgentProviderAttemptPhase::CountDriving, _) => {
                Err(AgentProviderAttemptStateError::CountRecoveryRequired)
            }
            (AgentProviderAttemptPhase::GenerationDriving, _) => {
                Err(AgentProviderAttemptStateError::GenerationRecoveryRequired)
            }
            _ => Err(AgentProviderAttemptStateError::Terminal),
        }
    }

    fn mark_count_drive_started(&mut self) -> Result<(), AgentProviderAttemptStateError> {
        match (&self.phase, self.core.as_ref()) {
            (
                AgentProviderAttemptPhase::Ready,
                Some(AgentProviderAttemptCore::PreDispatch { .. }),
            ) => {
                self.phase = AgentProviderAttemptPhase::CountDriving;
                Ok(())
            }
            (AgentProviderAttemptPhase::CountDriving, _) => {
                Err(AgentProviderAttemptStateError::CountRecoveryRequired)
            }
            (AgentProviderAttemptPhase::GenerationDriving, _) => {
                Err(AgentProviderAttemptStateError::GenerationRecoveryRequired)
            }
            _ => Err(AgentProviderAttemptStateError::Terminal),
        }
    }

    fn count(&self) -> Result<&AgentProviderExactInputCount, AgentProviderAttemptStateError> {
        match &self.phase {
            AgentProviderAttemptPhase::Counted(count) => Ok(count),
            AgentProviderAttemptPhase::CountDriving => {
                Err(AgentProviderAttemptStateError::CountRecoveryRequired)
            }
            AgentProviderAttemptPhase::GenerationDriving => {
                Err(AgentProviderAttemptStateError::GenerationRecoveryRequired)
            }
            AgentProviderAttemptPhase::Ready | AgentProviderAttemptPhase::Terminal => {
                Err(AgentProviderAttemptStateError::Terminal)
            }
        }
    }

    fn count_driving_committed(
        &self,
    ) -> Result<&AgentCommittedProviderRequest, AgentProviderAttemptStateError> {
        match (&self.phase, self.core.as_ref()) {
            (
                AgentProviderAttemptPhase::CountDriving,
                Some(AgentProviderAttemptCore::PreDispatch { committed, .. }),
            ) => Ok(committed),
            _ => Err(self.state_error()),
        }
    }

    fn mark_input_count_disclosed(&mut self) {
        if self.prior_disclosure_stage == AgentProviderDisclosureStage::NotDispatched {
            self.prior_disclosure_stage = AgentProviderDisclosureStage::InputTokenCountDisclosed;
        }
    }

    fn mark_model_may_have_dispatched(&mut self) {
        self.prior_disclosure_stage = AgentProviderDisclosureStage::ModelRequestMayHaveDispatched;
    }

    /// Exact non-authorizing call correlation while terminal authority remains.
    pub fn call(&self) -> Option<AgentProviderCallIdentity> {
        match self.core.as_ref() {
            Some(AgentProviderAttemptCore::PreDispatch { committed, .. }) => {
                Some(committed.request().call())
            }
            Some(AgentProviderAttemptCore::Generation(terminal)) => Some(
                AgentProviderCallIdentity::from_active(terminal.input.active()),
            ),
            None => None,
        }
    }

    /// Exact content-free semantic input proof committed at admission.
    ///
    /// Clone an observation proof before consuming the attempt when a future
    /// bounded continuation protocol needs an exact baseline. Losing this
    /// optional proof grants no authority and requires a fresh snapshot.
    pub fn input_evidence(&self) -> Option<&AgentProviderInputEvidence> {
        match self.core.as_ref() {
            Some(AgentProviderAttemptCore::PreDispatch { committed, .. }) => {
                Some(committed.input_evidence())
            }
            Some(AgentProviderAttemptCore::Generation(terminal)) => Some(terminal.input.evidence()),
            None => None,
        }
    }

    /// Content-free metrics for the exact input that crossed disclosure commit.
    pub fn input_metrics(&self) -> Option<AgentProviderInputMetrics> {
        match self.core.as_ref() {
            Some(AgentProviderAttemptCore::PreDispatch { committed, .. }) => {
                Some(committed.input_metrics())
            }
            Some(AgentProviderAttemptCore::Generation(terminal)) => Some(terminal.input.metrics()),
            None => None,
        }
    }

    /// Final input metrics when no provider count can still replace them.
    ///
    /// Exact-local attempts return one receipt. Provider-exact pre-count
    /// attempts return `None`; their receipt becomes available only through
    /// [`AgentProviderCountedAttempt`] or a terminal transport result.
    pub fn input_metric_receipt(&self) -> Option<AgentProviderInputMetricReceipt> {
        match self.core.as_ref() {
            Some(AgentProviderAttemptCore::PreDispatch { committed, .. }) => {
                committed.input_metric_receipt()
            }
            Some(AgentProviderAttemptCore::Generation(terminal)) => terminal.input.metric_receipt(),
            None => None,
        }
    }
}

impl AgentProviderAttempt {
    const fn phase_label(&self) -> &'static str {
        match self.phase {
            AgentProviderAttemptPhase::Ready => "ready",
            AgentProviderAttemptPhase::CountDriving => "count-driving",
            AgentProviderAttemptPhase::Counted(_) => "counted",
            AgentProviderAttemptPhase::GenerationDriving => "generation-driving",
            AgentProviderAttemptPhase::Terminal => "terminal",
        }
    }

    fn state_error(&self) -> AgentProviderAttemptStateError {
        match self.phase {
            AgentProviderAttemptPhase::CountDriving => {
                AgentProviderAttemptStateError::CountRecoveryRequired
            }
            AgentProviderAttemptPhase::GenerationDriving => {
                AgentProviderAttemptStateError::GenerationRecoveryRequired
            }
            AgentProviderAttemptPhase::Ready
            | AgentProviderAttemptPhase::Counted(_)
            | AgentProviderAttemptPhase::Terminal => AgentProviderAttemptStateError::Terminal,
        }
    }

    fn slot(&self) -> Option<&AgentProviderSlot> {
        match self.core.as_ref() {
            Some(AgentProviderAttemptCore::PreDispatch { slot, .. }) => slot.as_ref(),
            Some(AgentProviderAttemptCore::Generation(terminal)) => terminal.slot.as_ref(),
            None => None,
        }
    }

    fn fail_stop(&self) {
        if let Some(slot) = self.slot() {
            slot.fail_stop();
        } else {
            self.cancellation.cancel();
            self.shutdown.cancel();
        }
    }

    fn finish_terminal(
        &mut self,
        outcome: AgentProviderTransportOutcomeOwned,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError> {
        let core = self
            .core
            .take()
            .ok_or(AgentProviderAttemptStateError::Terminal)?;
        drop(self.credential.take());
        drop(self.model.take());
        self.phase = AgentProviderAttemptPhase::Terminal;
        let mut result = match core {
            AgentProviderAttemptCore::PreDispatch { committed, slot } => {
                let input_metric_receipt = committed.sealed_input_metric_receipt();
                let (request, input, continuation) = (*committed).into_parts();
                let (active, _) = input.into_parts();
                let (_, config, _, _) = request.into_transport_parts();
                finish_attempt(
                    active,
                    config,
                    outcome,
                    self.prior_disclosure_stage,
                    continuation,
                    input_metric_receipt,
                    slot,
                )
            }
            AgentProviderAttemptCore::Generation(terminal) => {
                let input_metric_receipt = terminal.input.sealed_metric_receipt();
                let (active, _) = terminal.input.into_parts();
                finish_attempt(
                    active,
                    terminal.config,
                    outcome,
                    self.prior_disclosure_stage,
                    terminal.continuation,
                    input_metric_receipt,
                    terminal.slot,
                )
            }
        };
        result.model_reply = self.model_reply.take();
        Ok(result)
    }

    fn counted_failure(
        &mut self,
        outcome: AgentProviderTransportOutcomeOwned,
    ) -> AgentProviderExactCountOutcome<'_> {
        match self.finish_terminal(outcome) {
            Ok(result) => AgentProviderExactCountOutcome::Failed(result),
            Err(error) => AgentProviderExactCountOutcome::Unavailable(error),
        }
    }

    fn counted_handle(
        &mut self,
    ) -> Result<AgentProviderCountedAttempt<'_>, AgentProviderAttemptStateError> {
        let count = self.count()?.clone();
        let input_metrics = self
            .input_metrics()
            .ok_or(AgentProviderAttemptStateError::Terminal)?;
        let input_metric_receipt = self
            .input_metric_receipt()
            .ok_or(AgentProviderAttemptStateError::Terminal)?;
        Ok(AgentProviderCountedAttempt {
            operation: self,
            count,
            input_metrics,
            input_metric_receipt,
        })
    }

    fn begin_generation_drive(
        &mut self,
    ) -> Result<
        (
            AgentProviderCallIdentity,
            AgentProviderCallConfig,
            AgentProviderEndpoint,
            Vec<u8>,
        ),
        AgentProviderAttemptStateError,
    > {
        match self.phase {
            AgentProviderAttemptPhase::Ready | AgentProviderAttemptPhase::Counted(_) => {}
            _ => return Err(self.state_error()),
        }
        let Some(core) = self.core.take() else {
            return Err(AgentProviderAttemptStateError::Terminal);
        };
        let AgentProviderAttemptCore::PreDispatch { committed, slot } = core else {
            self.core = Some(core);
            return Err(AgentProviderAttemptStateError::GenerationRecoveryRequired);
        };
        let (request, input, continuation) = (*committed).into_parts();
        let (call, config, endpoint_class, body) = request.into_transport_parts();
        self.core = Some(AgentProviderAttemptCore::Generation(Box::new(
            AgentProviderTerminalCore {
                input,
                config: config.clone(),
                continuation,
                slot,
            },
        )));
        // Persist ownership transfer before the borrowed drive can reach an
        // await point. A dropped drive is now abort-only, never replayable.
        self.phase = AgentProviderAttemptPhase::GenerationDriving;
        Ok((call, config, endpoint_class, body))
    }

    fn generation_matches(&self, call: AgentProviderCallIdentity) -> bool {
        matches!(
            self.core.as_ref(),
            Some(AgentProviderAttemptCore::Generation(terminal))
                if call.matches_active(terminal.input.active())
        )
    }

    /// Counts immutable input, with one bounded retry for explicit server overload.
    ///
    /// This borrows the operation. Dropping its future after it reaches the
    /// send point preserves the terminal authority for [`Self::abort`].
    pub async fn count_openai_input_tokens(&mut self) -> AgentProviderExactCountOutcome<'_> {
        if matches!(self.phase, AgentProviderAttemptPhase::Counted(_)) {
            return match self.counted_handle() {
                Ok(counted) => AgentProviderExactCountOutcome::Counted(counted),
                Err(error) => AgentProviderExactCountOutcome::Unavailable(error),
            };
        }
        if !matches!(self.phase, AgentProviderAttemptPhase::Ready) {
            return AgentProviderExactCountOutcome::Unavailable(self.state_error());
        }
        let Some(endpoint) = self.input_token_endpoint.clone() else {
            return self.counted_failure(protocol_failure());
        };
        let valid_count = self.provider == AgentProviderKind::OpenAiResponses
            && self
                .pre_dispatch()
                .map(|committed| {
                    committed.request().config().input_accounting_mode()
                        == crate::AgentProviderInputAccountingMode::ProviderExactAfterConservativeReservation
                })
                .unwrap_or(false);
        if !valid_count {
            return self.counted_failure(protocol_failure());
        }
        if self.cancellation.is_cancelled() || self.shutdown.is_cancelled() {
            return self.counted_failure(cancelled_failure());
        }
        if Instant::now() >= self.deadline {
            return self.counted_failure(timeout_failure());
        }
        let projection = match self.pre_dispatch().and_then(|committed| {
            committed
                .request()
                .openai_input_token_request()
                .map_err(|_| AgentProviderAttemptStateError::Terminal)
        }) {
            Ok(projection) => projection,
            Err(_) => return self.counted_failure(protocol_failure()),
        };
        let call = projection.call();
        let binding = projection.binding();
        let credential = match self
            .credential
            .as_ref()
            .ok_or(AgentProviderAttemptStateError::Terminal)
            .and_then(|credential| {
                credential
                    .sensitive_header(self.provider)
                    .map_err(|_| AgentProviderAttemptStateError::Terminal)
            }) {
            Ok(credential) => credential,
            Err(_) => return self.counted_failure(protocol_failure()),
        };
        let Some(request) = openai_input_token_request(
            &self.client,
            endpoint.clone(),
            call,
            credential,
            projection.into_body(),
        ) else {
            return self.counted_failure(protocol_failure());
        };
        if self.cancellation.is_cancelled() || self.shutdown.is_cancelled() {
            return self.counted_failure(cancelled_failure());
        }
        if Instant::now() >= self.deadline {
            return self.counted_failure(timeout_failure());
        }
        if self.mark_count_drive_started().is_err() {
            return AgentProviderExactCountOutcome::Unavailable(self.state_error());
        }
        let image_fence = self.image_fence.clone();
        if image_fence
            .as_ref()
            .is_some_and(|fence| !fence.while_current(&mut || {}))
        {
            return self.counted_failure(cancelled_failure());
        }
        // This is immediately before the first possible request.send() poll.
        // Count disclosure is monotonic but still proves zero model dispatch.
        self.mark_input_count_disclosed();
        let deadline = tokio::time::sleep_until(tokio::time::Instant::from_std(self.deadline));
        tokio::pin!(deadline);
        let mut retries = 0_u8;
        let response = loop {
            let Some(send) = request.try_clone() else {
                return self.counted_failure(protocol_failure());
            };
            let response = tokio::select! {
                biased;
                () = self.cancellation.cancelled() => return self.counted_failure(cancelled_failure()),
                () = self.shutdown.cancelled() => return self.counted_failure(cancelled_failure()),
                () = lead::image_fence::revoked(image_fence.as_ref()) => return self.counted_failure(cancelled_failure()),
                () = &mut deadline => return self.counted_failure(timeout_failure()),
                response = lead::image_fence::fenced(image_fence.as_ref(), send.send()) => match response { Ok(response) => response, Err(()) => return self.counted_failure(cancelled_failure()) },
            };
            let response = match response {
                Ok(response) => response,
                Err(error) => return self.counted_failure(network_failure(&error)),
            };
            if response.url() != &endpoint
                || !response_headers_admitted(response.headers())
                || !response_content_length_admitted(
                    response.headers(),
                    MAX_OPENAI_INPUT_TOKEN_RESPONSE_BYTES as u32,
                )
                || !response_encoding_admitted(response.headers())
            {
                return self.counted_failure(protocol_failure());
            }
            if response.status() == StatusCode::OK {
                break response;
            }
            let failure = status_failure(response.status(), response.headers());
            let jitter =
                openai_client_request_id(call, ProviderRequestPhase::InputTokens).map_or(0, |id| {
                    id.as_bytes()
                        .iter()
                        .fold(0_u64, |sum, byte| (sum * 31 + u64::from(*byte)) % 251)
                });
            let delay = Duration::from_millis(
                failure
                    .retry_after()
                    .map_or(500 + jitter, |hint| hint.millis()),
            );
            let retry = retries == 0
                && failure.class() == AgentProviderFailureClass::Overloaded
                && (!response.headers().contains_key(RETRY_AFTER)
                    || failure.retry_after().is_some())
                && Instant::now()
                    .checked_add(delay)
                    .is_some_and(|wake| wake < self.deadline);
            if !retry {
                return self.counted_failure(AgentProviderTransportOutcomeOwned::Failed(failure));
            }
            // Counting has dispatched no model or browser effect. Keep the same
            // bytes, admission, disclosure owner and absolute deadline.
            drop(response);
            retries += 1;
            tokio::select! {
                biased;
                () = self.cancellation.cancelled() => return self.counted_failure(cancelled_failure()),
                () = self.shutdown.cancelled() => return self.counted_failure(cancelled_failure()),
                () = lead::image_fence::revoked(image_fence.as_ref()) => return self.counted_failure(cancelled_failure()),
                () = &mut deadline => return self.counted_failure(timeout_failure()),
                () = tokio::time::sleep(delay) => {},
            }
        };
        if !response_json_content_type_admitted(response.headers()) {
            return self.counted_failure(protocol_failure());
        }
        let mut body = Vec::new();
        if body
            .try_reserve_exact(MAX_OPENAI_INPUT_TOKEN_RESPONSE_BYTES)
            .is_err()
        {
            return self.counted_failure(protocol_failure());
        }
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! {
                biased;
                () = self.cancellation.cancelled() => return self.counted_failure(cancelled_failure()),
                () = self.shutdown.cancelled() => return self.counted_failure(cancelled_failure()),
                () = lead::image_fence::revoked(image_fence.as_ref()) => return self.counted_failure(cancelled_failure()),
                () = &mut deadline => return self.counted_failure(timeout_failure()),
                next = stream.try_next() => next,
            };
            let chunk = match next {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => return self.counted_failure(network_failure(&error)),
            };
            let Some(next_len) = body.len().checked_add(chunk.len()) else {
                return self.counted_failure(protocol_failure());
            };
            if next_len > MAX_OPENAI_INPUT_TOKEN_RESPONSE_BYTES {
                return self.counted_failure(protocol_failure());
            }
            body.extend_from_slice(&chunk);
        }
        let tokens = match decode_openai_input_token_count(&body) {
            Some(tokens) => tokens,
            None => return self.counted_failure(protocol_failure()),
        };
        let count = match self.count_driving_committed().and_then(|committed| {
            AgentProviderExactInputCount::try_new(committed.request(), binding, tokens)
                .map_err(|_| AgentProviderAttemptStateError::Terminal)
        }) {
            Ok(count) => count,
            Err(_) => return self.counted_failure(protocol_failure()),
        };
        let bound = match self.core.as_mut() {
            Some(AgentProviderAttemptCore::PreDispatch { committed, .. }) => {
                committed.bind_provider_exact_input_count(count.clone())
            }
            _ => return AgentProviderExactCountOutcome::Unavailable(self.state_error()),
        };
        if bound.is_err() {
            return self.counted_failure(protocol_failure());
        }
        self.phase = AgentProviderAttemptPhase::Counted(count);
        match self.counted_handle() {
            Ok(counted) => AgentProviderExactCountOutcome::Counted(counted),
            Err(error) => AgentProviderExactCountOutcome::Unavailable(error),
        }
    }

    /// Consumes the retained terminal authority after a host-forced drive drop.
    ///
    /// The public reason only classifies the content-free failure. Exact-zero
    /// versus reservation-ceiling accounting derives exclusively from the
    /// private monotonic disclosure stage persisted by the drive.
    pub fn abort(
        mut self,
        reason: AgentProviderAbortReason,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError> {
        let outcome = match reason {
            AgentProviderAbortReason::HostDeadline => timeout_failure(),
            AgentProviderAbortReason::ControllerFault => integration_failure(),
        };
        self.finish_terminal(outcome)
    }

    /// Cancels before a generation request can have been dispatched.
    pub fn cancel_without_dispatch(
        &mut self,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError> {
        if matches!(self.phase, AgentProviderAttemptPhase::GenerationDriving) {
            return Err(AgentProviderAttemptStateError::GenerationRecoveryRequired);
        }
        if matches!(self.phase, AgentProviderAttemptPhase::Terminal) {
            return Err(AgentProviderAttemptStateError::Terminal);
        }
        self.finish_terminal(cancelled_failure())
    }

    /// Drives one Anthropic request while retaining all terminal authority.
    pub async fn execute_anthropic<F>(
        &mut self,
        consume: F,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError>
    where
        F: FnMut(AgentProviderStreamBatch) -> AgentProviderBatchDisposition,
    {
        if matches!(
            self.phase,
            AgentProviderAttemptPhase::CountDriving
                | AgentProviderAttemptPhase::GenerationDriving
                | AgentProviderAttemptPhase::Terminal
        ) {
            return Err(self.state_error());
        }
        if self.provider != AgentProviderKind::AnthropicMessages {
            return self.finish_terminal(protocol_failure());
        }
        self.execute_model(consume).await
    }

    /// Drives an OpenAI request admitted by pinned exact-local accounting.
    pub async fn execute_openai_exact_local<F>(
        &mut self,
        consume: F,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError>
    where
        F: FnMut(AgentProviderStreamBatch) -> AgentProviderBatchDisposition,
    {
        if matches!(
            self.phase,
            AgentProviderAttemptPhase::CountDriving
                | AgentProviderAttemptPhase::GenerationDriving
                | AgentProviderAttemptPhase::Terminal
        ) {
            return Err(self.state_error());
        }
        let exact_local = self.provider == AgentProviderKind::OpenAiResponses
            && self
                .pre_dispatch()
                .map(|committed| {
                    matches!(
                        committed.request().config().input_accounting_mode(),
                        crate::AgentProviderInputAccountingMode::ExactLocal { .. }
                    )
                })
                .unwrap_or(false);
        if !exact_local {
            return self.finish_terminal(protocol_failure());
        }
        self.execute_model(consume).await
    }

    #[cfg(test)]
    async fn execute<F>(&mut self, consume: F) -> AgentProviderTransportResult
    where
        F: FnMut(AgentProviderStreamBatch) -> AgentProviderBatchDisposition,
    {
        self.execute_model(consume)
            .await
            .expect("test operation state")
    }

    async fn execute_model<F>(
        &mut self,
        mut consume: F,
    ) -> Result<AgentProviderTransportResult, AgentProviderAttemptStateError>
    where
        F: FnMut(AgentProviderStreamBatch) -> AgentProviderBatchDisposition,
    {
        let (call, config, endpoint_class, body) = self.begin_generation_drive()?;
        if !self.generation_matches(call)
            || !provider_endpoint_matches(self.provider, endpoint_class)
        {
            return self.finish_terminal(protocol_failure());
        }
        if Instant::now() >= self.deadline {
            return self.finish_terminal(timeout_failure());
        }
        if self.cancellation.is_cancelled() || self.shutdown.is_cancelled() {
            return self.finish_terminal(cancelled_failure());
        }
        let mut decoder = match AgentProviderStreamDecoder::try_new(call, &config) {
            Ok(decoder) => decoder,
            Err(error) => return self.finish_terminal(protocol_decoder_failure(error, None)),
        };
        let credential = match self
            .credential
            .as_ref()
            .ok_or(AgentProviderAttemptStateError::Terminal)
            .and_then(|credential| {
                credential
                    .sensitive_header(self.provider)
                    .map_err(|_| AgentProviderAttemptStateError::Terminal)
            }) {
            Ok(credential) => credential,
            Err(_) => return self.finish_terminal(protocol_failure()),
        };
        let Some(request) = provider_request(
            &self.client,
            self.endpoint.clone(),
            self.provider,
            call,
            credential,
            body,
        ) else {
            return self.finish_terminal(protocol_failure());
        };
        if self.cancellation.is_cancelled() || self.shutdown.is_cancelled() {
            return self.finish_terminal(cancelled_failure());
        }
        if Instant::now() >= self.deadline {
            return self.finish_terminal(timeout_failure());
        }
        let image_fence = self.image_fence.clone();
        if image_fence
            .as_ref()
            .is_some_and(|fence| !fence.while_current(&mut || {}))
        {
            return self.finish_terminal(cancelled_failure());
        }
        // This is immediately before the first possible generation send poll.
        self.mark_model_may_have_dispatched();
        let deadline = tokio::time::sleep_until(tokio::time::Instant::from_std(self.deadline));
        tokio::pin!(deadline);
        let response = tokio::select! {
            biased;
            () = self.cancellation.cancelled() => return self.finish_terminal(cancelled_failure()),
            () = self.shutdown.cancelled() => return self.finish_terminal(cancelled_failure()),
            () = lead::image_fence::revoked(image_fence.as_ref()) => return self.finish_terminal(cancelled_failure()),
            () = &mut deadline => return self.finish_terminal(timeout_failure()),
            response = lead::image_fence::fenced(image_fence.as_ref(), request.send()) => match response { Ok(response) => response, Err(()) => return self.finish_terminal(cancelled_failure()) },
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => return self.finish_terminal(network_failure(&error)),
        };
        if response.url() != &self.endpoint || !response_headers_admitted(response.headers()) {
            return self.finish_terminal(protocol_failure());
        }
        if response.status() != StatusCode::OK {
            let failure = status_failure(response.status(), response.headers());
            return self.finish_terminal(AgentProviderTransportOutcomeOwned::Failed(failure));
        }
        if !response_content_length_admitted(
            response.headers(),
            config.stream_budget().max_wire_bytes(),
        ) || !response_encoding_admitted(response.headers())
            || !response_content_type_admitted(response.headers())
        {
            return self.finish_terminal(protocol_failure());
        }
        let mut stream = response.bytes_stream();
        loop {
            let next = tokio::select! {
                biased;
                () = self.cancellation.cancelled() => return self.finish_terminal(cancelled_failure()),
                () = self.shutdown.cancelled() => return self.finish_terminal(cancelled_failure()),
            () = lead::image_fence::revoked(image_fence.as_ref()) => return self.finish_terminal(cancelled_failure()),
                () = &mut deadline => return self.finish_terminal(timeout_failure()),
                next = stream.try_next() => next,
            };
            let chunk = match next {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => return self.finish_terminal(network_failure(&error)),
            };
            let batch = match decoder.push(&chunk) {
                Ok(batch) => batch,
                Err(error) => {
                    let event = decoder.protocol_event();
                    return self.finish_terminal(protocol_decoder_failure(error, event));
                }
            };
            if !batch.deltas().is_empty() {
                let disposition =
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| consume(batch)))
                    {
                        Ok(disposition) => disposition,
                        Err(_) => {
                            self.fail_stop();
                            return self.finish_terminal(integration_failure());
                        }
                    };
                if disposition == AgentProviderBatchDisposition::Cancel {
                    return self.finish_terminal(cancelled_failure());
                }
            }
        }
        // Transport-observed body EOF is the completion linearization point.
        // Cancellation wins while another body item is pending; once EOF wins,
        // later cancellation cannot erase the terminal. Pricing and controller
        // policy checks remain required before any native effect can occur.
        let protocol_event = decoder.protocol_event();
        let outcome = match decoder.finish() {
            Ok(finished) => AgentProviderTransportOutcomeOwned::Stream(Box::new(finished)),
            Err(error) => protocol_decoder_failure(error, protocol_event),
        };
        self.finish_terminal(outcome)
    }
}

impl Drop for AgentProviderAttempt {
    fn drop(&mut self) {
        if self.core.is_some() {
            // An abandoned operation cannot look clean: seal the shared slot
            // before its move-only policy authority is dropped.
            self.fail_stop();
            self.phase = AgentProviderAttemptPhase::Terminal;
        }
    }
}

impl fmt::Debug for AgentProviderAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentProviderAttempt")
            .field("call", &self.call())
            .field("provider", &self.provider)
            .field("endpoint", &"[fixed]")
            .field("credential", &"[redacted]")
            .field("model", &self.model.is_some())
            .field("phase", &self.phase_label())
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish()
    }
}

#[derive(Clone, Copy)]
enum ProviderRequestPhase {
    InputTokens,
    Generation,
}

impl ProviderRequestPhase {
    const fn tag(self) -> u8 {
        match self {
            Self::InputTokens => 1,
            Self::Generation => 2,
        }
    }
}

fn openai_client_request_id(
    call: AgentProviderCallIdentity,
    phase: ProviderRequestPhase,
) -> Option<HeaderValue> {
    let mut hasher = Sha256::new();
    hasher.update(OPENAI_CLIENT_REQUEST_ID_DOMAIN);
    hasher.update(call.manifest().bytes());
    hasher.update(call.call().get().to_be_bytes());
    hasher.update([phase.tag()]);
    let digest: [u8; 32] = hasher.finalize().into();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    HeaderValue::from_str(&format!("zephium-v1-{encoded}")).ok()
}

fn provider_request(
    client: &Client,
    endpoint: Url,
    provider: AgentProviderKind,
    call: AgentProviderCallIdentity,
    credential: HeaderValue,
    body: Vec<u8>,
) -> Option<reqwest::RequestBuilder> {
    #[cfg(feature = "probe-harness")]
    if serde_json::from_slice::<serde_json::Value>(&body)
        .is_ok_and(|body| body.get("store") == Some(&serde_json::Value::Bool(true)))
    {
        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let index = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 128;
        let directory = "target/work-runtime-proof/native-requests";
        let _ = std::fs::create_dir_all(directory);
        let _ = std::fs::write(format!("{directory}/{index:03}.json"), &body);
    }
    let request = client
        .post(endpoint)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .header(ACCEPT, HeaderValue::from_static("text/event-stream"))
        .header(ACCEPT_ENCODING, HeaderValue::from_static("identity"))
        .header(CACHE_CONTROL, HeaderValue::from_static("no-store"))
        .body(body);
    match provider {
        AgentProviderKind::OpenAiResponses => {
            Some(request.header(AUTHORIZATION, credential).header(
                HeaderName::from_static(OPENAI_CLIENT_REQUEST_ID_HEADER),
                openai_client_request_id(call, ProviderRequestPhase::Generation)?,
            ))
        }
        AgentProviderKind::AnthropicMessages => Some(
            request
                .header(HeaderName::from_static("x-api-key"), credential)
                .header(
                    HeaderName::from_static("anthropic-version"),
                    HeaderValue::from_static(ANTHROPIC_VERSION),
                ),
        ),
    }
}

fn openai_input_token_request(
    client: &Client,
    endpoint: Url,
    call: AgentProviderCallIdentity,
    credential: HeaderValue,
    body: Vec<u8>,
) -> Option<reqwest::RequestBuilder> {
    Some(
        client
            .post(endpoint)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .header(ACCEPT_ENCODING, HeaderValue::from_static("identity"))
            .header(CACHE_CONTROL, HeaderValue::from_static("no-store"))
            .header(AUTHORIZATION, credential)
            .header(
                HeaderName::from_static(OPENAI_CLIENT_REQUEST_ID_HEADER),
                openai_client_request_id(call, ProviderRequestPhase::InputTokens)?,
            )
            .body(body),
    )
}

fn decode_openai_input_token_count(bytes: &[u8]) -> Option<u32> {
    let serde_json::Value::Object(object) = serde_json::from_slice(bytes).ok()? else {
        return None;
    };
    if object.len() != 2
        || object.get("object")?.as_str()? != "response.input_tokens"
        || !object.contains_key("input_tokens")
    {
        return None;
    }
    u32::try_from(object.get("input_tokens")?.as_u64()?).ok()
}

fn finish_attempt(
    active: AgentActiveModelCall,
    config: AgentProviderCallConfig,
    mut outcome: AgentProviderTransportOutcomeOwned,
    mut disclosure_stage: AgentProviderDisclosureStage,
    mut continuation: Option<AgentProviderContinuationSeed>,
    input_metric_receipt: AgentProviderInputMetricReceipt,
    mut slot: Option<AgentProviderSlot>,
) -> AgentProviderTransportResult {
    if let AgentProviderTransportOutcomeOwned::Stream(finished) = &outcome {
        let call = match finished.conclusion() {
            AgentProviderStreamConclusion::Completed(completion) => completion.call(),
            AgentProviderStreamConclusion::Failed(failure) => failure.call(),
        };
        if !call.matches_active(&active) {
            outcome = protocol_failure();
            disclosure_stage = AgentProviderDisclosureStage::ModelRequestMayHaveDispatched;
            continuation = None;
        }
    }
    if !matches!(
        &outcome,
        AgentProviderTransportOutcomeOwned::Stream(finished)
            if matches!(finished.conclusion(), AgentProviderStreamConclusion::Completed(
                completion
            ) if completion.stop() == AgentProviderStopReason::ToolCalls
            && completion.tool_only_output()
            && completion.stats().tool_calls() == 1)
    ) {
        continuation = None;
    }
    if let Some(slot) = &mut slot {
        slot.mark_completed();
    }
    drop(slot);
    AgentProviderTransportResult {
        active,
        config,
        input_metric_receipt,
        outcome,
        disclosure_stage,
        continuation: continuation.map(Box::new),
        model_reply: None,
    }
}

const fn settlement_for_failure(failure: AgentProviderFailure) -> AgentModelCallSettlement {
    if matches!(failure.class(), AgentProviderFailureClass::Cancelled) {
        AgentModelCallSettlement::Cancelled
    } else {
        AgentModelCallSettlement::ProviderFailed
    }
}

const fn unaccounted_settlement_for_failure(
    failure: AgentProviderFailure,
) -> AgentModelCallUnaccountedSettlement {
    if matches!(failure.class(), AgentProviderFailureClass::Cancelled) {
        AgentModelCallUnaccountedSettlement::Cancelled
    } else {
        AgentModelCallUnaccountedSettlement::ProviderFailed
    }
}

fn cancelled_failure() -> AgentProviderTransportOutcomeOwned {
    AgentProviderTransportOutcomeOwned::Failed(AgentProviderFailure::new(
        AgentProviderFailureClass::Cancelled,
    ))
}

const fn protocol_failure_value() -> AgentProviderFailure {
    AgentProviderFailure::new(AgentProviderFailureClass::Protocol)
}

fn protocol_failure() -> AgentProviderTransportOutcomeOwned {
    AgentProviderTransportOutcomeOwned::Failed(protocol_failure_value())
}

fn protocol_decoder_failure(
    error: crate::AgentProviderProtocolError,
    event: Option<crate::AgentProviderProtocolEvent>,
) -> AgentProviderTransportOutcomeOwned {
    AgentProviderTransportOutcomeOwned::Failed(AgentProviderFailure::protocol_at(error, event))
}

fn integration_failure() -> AgentProviderTransportOutcomeOwned {
    AgentProviderTransportOutcomeOwned::Failed(AgentProviderFailure::new(
        AgentProviderFailureClass::Integration,
    ))
}

fn timeout_failure() -> AgentProviderTransportOutcomeOwned {
    AgentProviderTransportOutcomeOwned::Failed(AgentProviderFailure::new(
        AgentProviderFailureClass::Timeout,
    ))
}

fn network_failure(error: &reqwest::Error) -> AgentProviderTransportOutcomeOwned {
    let class = if error.is_timeout() {
        AgentProviderFailureClass::Timeout
    } else {
        AgentProviderFailureClass::Transport
    };
    AgentProviderTransportOutcomeOwned::Failed(AgentProviderFailure::new(class))
}

fn status_failure(status: StatusCode, headers: &HeaderMap) -> AgentProviderFailure {
    let class = match status.as_u16() {
        401 => AgentProviderFailureClass::Authentication,
        403 => AgentProviderFailureClass::Permission,
        404 => AgentProviderFailureClass::NotFound,
        408 => AgentProviderFailureClass::Timeout,
        409 => AgentProviderFailureClass::Conflict,
        429 => AgentProviderFailureClass::RateLimited,
        400..=499 => AgentProviderFailureClass::InvalidRequest,
        500 | 502 | 503 | 504 | 529 => AgentProviderFailureClass::Overloaded,
        300..=399 => AgentProviderFailureClass::Protocol,
        _ => AgentProviderFailureClass::Provider,
    };
    let retry_after = if matches!(
        class,
        AgentProviderFailureClass::RateLimited
            | AgentProviderFailureClass::Overloaded
            | AgentProviderFailureClass::Timeout
            | AgentProviderFailureClass::Transport
    ) {
        parse_retry_after(headers)
    } else {
        None
    };
    AgentProviderFailure::try_new(class, retry_after)
        .unwrap_or_else(|_| AgentProviderFailure::new(class))
}

fn parse_retry_after(headers: &HeaderMap) -> Option<AgentProviderRetryAfter> {
    let mut values = headers.get_all(RETRY_AFTER).iter();
    let bytes = match (values.next(), values.next()) {
        (Some(value), None) => value.as_bytes(),
        _ => return None,
    };
    if bytes.is_empty()
        || (bytes.len() > 1 && bytes[0] == b'0')
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let seconds = bytes.iter().try_fold(0_u64, |value, byte| {
        value
            .checked_mul(10)?
            .checked_add(u64::from(byte.saturating_sub(b'0')))
    })?;
    let millis = seconds.checked_mul(1_000)?;
    AgentProviderRetryAfter::try_from_millis(millis).ok()
}

/// Applies one protocol-neutral decoded header-list ceiling before any status,
/// retry hint, content-type, or body processing. Reqwest forwards the same
/// limit to HTTP/2 before decode; pinned reqwest does not expose Hyper's HTTP/1
/// receive-buffer ceiling, so this additional checked pass is also mandatory
/// after an HTTP/1 response has been parsed.
fn response_headers_admitted(headers: &HeaderMap) -> bool {
    headers
        .iter()
        .try_fold(0_usize, |total, (name, value)| {
            total
                .checked_add(name.as_str().len())?
                .checked_add(value.as_bytes().len())?
                .checked_add(HTTP_HEADER_FIELD_OVERHEAD_BYTES)
        })
        .is_some_and(
            |bytes| match usize::try_from(MAX_AGENT_PROVIDER_RESPONSE_HEADER_BYTES) {
                Ok(limit) => bytes <= limit,
                Err(_) => false,
            },
        )
}

fn response_encoding_admitted(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(CONTENT_ENCODING).iter();
    match (values.next(), values.next()) {
        (None, None) => true,
        (Some(value), None) => value.as_bytes().eq_ignore_ascii_case(b"identity"),
        _ => false,
    }
}

/// Refuses a declared success body larger than this exact call's wire budget
/// before the response body stream can allocate or yield any bytes. Streaming
/// responses may omit the field, but an ambiguous, non-canonical, or duplicate
/// declaration is never trusted as a resource bound.
fn response_content_length_admitted(headers: &HeaderMap, max_wire_bytes: u32) -> bool {
    let mut values = headers.get_all(CONTENT_LENGTH).iter();
    let bytes = match (values.next(), values.next()) {
        (None, None) => return true,
        (Some(value), None) => value.as_bytes(),
        _ => return false,
    };
    if bytes.is_empty()
        || (bytes.len() > 1 && bytes[0] == b'0')
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    bytes
        .iter()
        .try_fold(0_u64, |value, byte| {
            value
                .checked_mul(10)?
                .checked_add(u64::from(byte.saturating_sub(b'0')))
        })
        .is_some_and(|declared| declared <= u64::from(max_wire_bytes))
}

fn response_content_type_admitted(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(CONTENT_TYPE).iter();
    let value = match (values.next(), values.next()) {
        (Some(value), None) => match value.to_str() {
            Ok(value) => value,
            Err(_) => return false,
        },
        _ => return false,
    };
    let mut parts = value.split(';').map(str::trim);
    if !parts
        .next()
        .is_some_and(|mime| mime.eq_ignore_ascii_case("text/event-stream"))
    {
        return false;
    }
    match (parts.next(), parts.next()) {
        (None, None) => true,
        (Some(parameter), None) => parameter.eq_ignore_ascii_case("charset=utf-8"),
        _ => false,
    }
}

fn response_json_content_type_admitted(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(CONTENT_TYPE).iter();
    let value = match (values.next(), values.next()) {
        (Some(value), None) => match value.to_str() {
            Ok(value) => value,
            Err(_) => return false,
        },
        _ => return false,
    };
    let mut parts = value.split(';').map(str::trim);
    if !parts
        .next()
        .is_some_and(|mime| mime.eq_ignore_ascii_case("application/json"))
    {
        return false;
    }
    match (parts.next(), parts.next()) {
        (None, None) => true,
        (Some(parameter), None) => parameter.eq_ignore_ascii_case("charset=utf-8"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc::{self, Receiver, SyncSender};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use crate::{
        compute_semantic_diff, decode_semantic_snapshot, encode_semantic_diff,
        encode_semantic_extraction_request, encode_semantic_observation, read_semantic_observation,
        AgentAccountAttestationId, AgentAccountScope, AgentBrowserToolKind,
        AgentContextAccountBinding, AgentDelegationSpec, AgentDelegationTopology, AgentEffectScope,
        AgentModelCallBudget, AgentModelCallId, AgentModelCallRequest, AgentModelCallSettlement,
        AgentPlanLeaseBinding, AgentPlanLeaseId, AgentPlanNodeAuthority, AgentPlanNodeId,
        AgentPlanNodeScope, AgentPolicyInstant, AgentPreparedObservationRequest,
        AgentProviderCallConfig, AgentProviderDiffRequestDraft,
        AgentProviderExtractionRequestDraft, AgentProviderInputKind,
        AgentProviderLocalInputTokenCounter, AgentProviderModelRevision, AgentProviderObjective,
        AgentProviderPricingContractError, AgentProviderPricingError, AgentProviderPricingProfile,
        AgentProviderPricingRevision, AgentProviderPricingSchedule, AgentProviderReasoningEffort,
        AgentProviderResponseRoute, AgentProviderRetryDisposition, AgentProviderSemanticInputStats,
        AgentProviderStopReason, AgentProviderStreamBudget, AgentProviderTokenRates,
        AgentRunBudget, AgentRunManifest, AgentRunManifestId, AgentRunProviderInputMetrics,
        AgentRunScope, AgentRunSupervisor, AgentSupervisorId, ContextCapabilities,
        ContextCapability, ContextId, ContextIdentity, ContextKind, ContextOperationId,
        ContextRegistry, ContextRunId, ContextSettlement, FrameGeneration, FrameId,
        SemanticCaptureInstant, SemanticDecodeContext, SemanticDiffBudget, SemanticDiffOutcome,
        SemanticEffectClass, SemanticExtractionFieldSchema, SemanticExtractionSchema,
        SemanticExtractionSchemaId, SemanticFrameJoin, SemanticFrameTrust, SemanticInvocationId,
        SemanticModelEncodingBudget, SemanticObservation, SemanticObservationAssembler,
        SemanticObservationBudget, SemanticObservationId, SemanticObservationRequest,
        SemanticOrigin, SemanticReadAuthority, SemanticReadBudget, SemanticReadSensitivityLimit,
        SemanticSensitivity, SemanticSnapshotGeneration, SemanticTokenCountQuality,
        SemanticTokenCountRequirement, SemanticTokenCounter, SemanticTokenCounterError,
        SemanticTokenMeasurement, SemanticTokenizerRevision, MAX_AGENT_PROVIDER_REQUEST_BYTES,
        SEMANTIC_WIRE_VERSION,
    };
    use serde_json::json;
    use zephium_core::ids::ProfileId;

    use super::*;

    const NOW: u64 = 2_000;
    const EXPIRES_AT: u64 = 100_000;

    trait TestTransportResultExt {
        fn disclosure_stage(&self) -> AgentProviderDisclosureStage;
        fn input_metric_receipt(&self) -> AgentProviderInputMetricReceipt;
        fn outcome(&self) -> AgentProviderTransportOutcome<'_>;
        fn usage_knowledge(&self) -> AgentProviderUsageKnowledge;
        fn into_policy_settlement(self) -> AgentProviderPolicySettlement;
    }

    impl TestTransportResultExt
        for Result<AgentProviderTransportResult, AgentProviderAttemptStateError>
    {
        fn disclosure_stage(&self) -> AgentProviderDisclosureStage {
            self.as_ref()
                .expect("test operation state")
                .disclosure_stage()
        }

        fn input_metric_receipt(&self) -> AgentProviderInputMetricReceipt {
            self.as_ref()
                .expect("test operation state")
                .input_metric_receipt()
        }

        fn outcome(&self) -> AgentProviderTransportOutcome<'_> {
            self.as_ref().expect("test operation state").outcome()
        }

        fn usage_knowledge(&self) -> AgentProviderUsageKnowledge {
            self.as_ref()
                .expect("test operation state")
                .usage_knowledge()
        }

        fn into_policy_settlement(self) -> AgentProviderPolicySettlement {
            self.expect("test operation state").into_policy_settlement()
        }
    }

    struct FixedCounter {
        revision: SemanticTokenizerRevision,
        tokens: u32,
    }

    impl SemanticTokenCounter for FixedCounter {
        fn count_tokens(
            &self,
            input: &str,
        ) -> Result<SemanticTokenMeasurement, SemanticTokenCounterError> {
            if input.is_empty() {
                return Err(SemanticTokenCounterError::InvalidResult);
            }
            SemanticTokenMeasurement::try_new(
                self.revision.clone(),
                self.tokens,
                SemanticTokenCountQuality::ExactLocal,
            )
            .map_err(|_| SemanticTokenCounterError::InvalidResult)
        }
    }

    struct FixedStructuredCounter {
        revision: SemanticTokenizerRevision,
        tokens: u32,
    }

    impl FixedStructuredCounter {
        fn count(
            &self,
            tokenizer: &SemanticTokenizerRevision,
            request_body: &[u8],
        ) -> Result<SemanticTokenMeasurement, SemanticTokenCounterError> {
            if tokenizer != &self.revision || request_body.is_empty() {
                return Err(SemanticTokenCounterError::InvalidResult);
            }
            SemanticTokenMeasurement::try_new(
                self.revision.clone(),
                self.tokens,
                SemanticTokenCountQuality::ExactLocal,
            )
            .map_err(|_| SemanticTokenCounterError::InvalidResult)
        }
    }

    impl AgentProviderLocalInputTokenCounter for FixedStructuredCounter {
        fn count_openai_responses_input(
            &self,
            model: &AgentProviderModelRevision,
            tokenizer: &SemanticTokenizerRevision,
            request_body: &[u8],
        ) -> Result<SemanticTokenMeasurement, SemanticTokenCounterError> {
            if model.as_str() != "gpt-5.6-terra" {
                return Err(SemanticTokenCounterError::Unavailable);
            }
            self.count(tokenizer, request_body)
        }

        fn count_anthropic_messages_input(
            &self,
            model: &AgentProviderModelRevision,
            tokenizer: &SemanticTokenizerRevision,
            request_body: &[u8],
        ) -> Result<SemanticTokenMeasurement, SemanticTokenCounterError> {
            if model.as_str() != "claude-opus-5" {
                return Err(SemanticTokenCounterError::Unavailable);
            }
            self.count(tokenizer, request_body)
        }
    }

    struct StatefulProviderFixture {
        policy: AgentRunPolicy,
        input: AgentProviderTransportInput,
        observation: SemanticObservation,
        account: AgentContextAccountBinding,
        lease: AgentPlanLeaseId,
        config: AgentProviderCallConfig,
    }

    fn provider_fixture(
        provider: AgentProviderKind,
    ) -> (AgentRunPolicy, AgentProviderTransportInput) {
        let fixture = stateful_provider_fixture(provider);
        (fixture.policy, fixture.input)
    }

    pub(super) fn provider_exact_fixture() -> (AgentRunPolicy, AgentProviderTransportInput) {
        let fixture = stateful_provider_fixture_with_accounting(
            AgentProviderKind::OpenAiResponses,
            AgentProviderPricingProfile::try_new(
                AgentProviderPricingRevision::new(1).expect("pricing revision"),
                crate::MAX_AGENT_PROVIDER_EXACT_COUNTED_INPUT_TOKENS,
            )
            .expect("pricing profile"),
            true,
        );
        (fixture.policy, fixture.input)
    }

    fn provider_input_metrics(
        policy: &AgentRunPolicy,
        supervisor_id: u64,
    ) -> AgentRunProviderInputMetrics {
        let topology = AgentDelegationTopology::try_new(
            policy.manifest(),
            policy
                .manifest()
                .plan_nodes()
                .iter()
                .map(|node| AgentDelegationSpec::new(node.id(), None))
                .collect(),
        )
        .expect("provider metric topology");
        let supervisor = AgentRunSupervisor::new(
            AgentSupervisorId::new(supervisor_id).expect("provider metric supervisor"),
            topology,
        );
        AgentRunProviderInputMetrics::try_new(policy.manifest(), &supervisor)
            .expect("provider input metrics")
    }

    fn stateful_provider_fixture(provider: AgentProviderKind) -> StatefulProviderFixture {
        stateful_provider_fixture_with_profile(
            provider,
            AgentProviderPricingProfile::try_new(
                AgentProviderPricingRevision::new(1).expect("pricing revision"),
                16_384,
            )
            .expect("pricing profile"),
        )
    }

    fn stateful_provider_fixture_with_profile(
        provider: AgentProviderKind,
        pricing_profile: AgentProviderPricingProfile,
    ) -> StatefulProviderFixture {
        stateful_provider_fixture_with_accounting(provider, pricing_profile, false)
    }

    fn stateful_provider_fixture_with_accounting(
        provider: AgentProviderKind,
        pricing_profile: AgentProviderPricingProfile,
        provider_exact: bool,
    ) -> StatefulProviderFixture {
        stateful_provider_fixture_for(provider, pricing_profile, provider_exact, None)
    }

    /// The provider-exact fixture's run, admitted for a chosen model instead.
    pub(super) fn model_reported_fixture(
        model: &model_client::AgentProviderModelClient,
    ) -> (AgentRunPolicy, AgentProviderTransportInput) {
        let fixture = stateful_provider_fixture_for(
            AgentProviderKind::OpenAiResponses,
            AgentProviderPricingProfile::try_new(
                AgentProviderPricingRevision::new(1).expect("pricing revision"),
                crate::MAX_AGENT_PROVIDER_EXACT_COUNTED_INPUT_TOKENS,
            )
            .expect("pricing profile"),
            true,
            Some(model),
        );
        (fixture.policy, fixture.input)
    }

    fn stateful_provider_fixture_for(
        provider: AgentProviderKind,
        pricing_profile: AgentProviderPricingProfile,
        provider_exact: bool,
        chosen: Option<&model_client::AgentProviderModelClient>,
    ) -> StatefulProviderFixture {
        assert!(
            !provider_exact || provider == AgentProviderKind::OpenAiResponses,
            "provider-exact fixture is OpenAI-only"
        );
        let run = ContextRunId::generate();
        let profile = ProfileId::generate();
        let identity =
            ContextIdentity::new(ContextId::generate(), run, profile, ContextKind::Owned);
        let capabilities = ContextCapabilities::try_new(
            ContextKind::Owned,
            &[ContextCapability::Observe, ContextCapability::Act],
        )
        .expect("capabilities");
        let mut registry = ContextRegistry::new();
        registry.reserve(identity, capabilities).expect("reserve");
        let operation = registry
            .begin_context(
                identity.id(),
                ContextOperationId::new(1).expect("operation"),
            )
            .expect("begin context");
        registry
            .settle_construction(identity.id(), operation, ContextSettlement::Applied)
            .expect("construct context");
        let context = registry.join(identity.id()).expect("context join");
        let origin = SemanticOrigin::parse("https://fixture.example.test/page").expect("origin");
        let frame = SemanticFrameJoin::try_new(
            context,
            FrameId::MAIN,
            FrameGeneration::INITIAL,
            origin.clone(),
            SemanticFrameTrust::SameOrigin,
        )
        .expect("frame");
        let invocation = SemanticInvocationId::new(11).expect("invocation");
        let wire = serde_json::to_vec(&json!({
            "v": SEMANTIC_WIRE_VERSION,
            "i": invocation.get(),
            "g": 11,
            "c": "complete",
            "n": [
                {"k": 1, "r": "document", "o": 16},
                {"k": 2, "p": 0, "r": "paragraph", "t": "synthetic fixture marker"}
            ]
        }))
        .expect("wire");
        let snapshot = decode_semantic_snapshot(
            SemanticDecodeContext::new(
                invocation,
                frame,
                SemanticSnapshotGeneration::new(11).expect("snapshot generation"),
            ),
            &wire,
        )
        .expect("snapshot");
        let observation = SemanticObservationAssembler::new(
            SemanticObservationRequest::initial(
                SemanticObservationId::new(1).expect("observation"),
                context,
                SemanticObservationBudget::try_new(8, 4_096, 1).expect("observation budget"),
            ),
            snapshot,
        )
        .expect("observation assembler")
        .finish()
        .expect("observation");
        let tokenizer = match chosen {
            Some(model) => model
                .try_call_config(20)
                .expect("chosen model config")
                .tokenizer()
                .clone(),
            None => SemanticTokenizerRevision::try_new("transport-test-v1".to_owned())
                .expect("tokenizer"),
        };
        let payload = if provider_exact {
            encode_semantic_observation(
                &observation,
                SemanticModelEncodingBudget::INITIAL_CONSERVATIVE,
            )
            .expect("encode")
            .admit_conservative_utf8(&tokenizer)
            .expect("conservative payload")
        } else {
            encode_semantic_observation(
                &observation,
                SemanticModelEncodingBudget::try_new(
                    8_192,
                    1_000,
                    SemanticTokenCountRequirement::Exact,
                )
                .expect("encoding budget"),
            )
            .expect("encode")
            .admit(
                &FixedCounter {
                    revision: tokenizer.clone(),
                    tokens: 10,
                },
                &tokenizer,
            )
            .expect("payload")
        };
        let objective = if provider_exact {
            AgentProviderObjective::try_admit_conservative_utf8(
                "Summarize the synthetic marker".to_owned(),
                &tokenizer,
            )
            .expect("conservative objective")
        } else {
            AgentProviderObjective::try_admit(
                "Summarize the synthetic marker".to_owned(),
                &FixedCounter {
                    revision: tokenizer.clone(),
                    tokens: 3,
                },
                &tokenizer,
            )
            .expect("objective")
        };
        let effects = AgentEffectScope::try_new(&[SemanticEffectClass::Read]).expect("effects");
        let run_budget = AgentRunBudget::try_new(
            8,
            if provider_exact {
                crate::MAX_AGENT_PROVIDER_EXACT_COUNTED_INPUT_TOKENS + 20
            } else {
                1_000
            },
            10_000,
            1,
        )
        .expect("run budget");
        let scope = AgentRunScope::try_new(
            vec![profile],
            vec![AgentAccountScope::Anonymous],
            vec![origin.clone()],
            SemanticSensitivity::Public,
            effects,
            Vec::new(),
        )
        .expect("scope");
        let node = AgentPlanNodeId::generate();
        let authority = AgentPlanNodeAuthority::try_new(
            vec![profile],
            vec![AgentAccountScope::Anonymous],
            vec![origin],
            SemanticSensitivity::Public,
            effects,
        )
        .expect("authority");
        let manifest = AgentRunManifest::try_new(
            AgentRunManifestId::generate(),
            run,
            scope,
            run_budget,
            AgentPolicyInstant::from_millis(1_000),
            AgentPolicyInstant::from_millis(EXPIRES_AT),
            vec![AgentPlanNodeScope::new(
                node,
                authority,
                run_budget,
                AgentPolicyInstant::from_millis(EXPIRES_AT - 1),
            )],
        )
        .expect("manifest");
        let lease = AgentPlanLeaseId::generate();
        let mut policy =
            AgentRunPolicy::try_new(manifest, vec![AgentPlanLeaseBinding::new(lease, node)])
                .expect("policy");
        let account = AgentContextAccountBinding::new(
            AgentAccountAttestationId::generate(),
            context,
            AgentAccountScope::Anonymous,
            AgentPolicyInstant::from_millis(NOW - 1),
        );
        let request = AgentModelCallRequest::new(
            AgentModelCallId::new(1).expect("call"),
            lease,
            account,
            AgentModelCallBudget::try_new(
                if provider_exact {
                    u32::try_from(crate::MAX_AGENT_PROVIDER_EXACT_COUNTED_INPUT_TOKENS)
                        .expect("provider-exact fixture ceiling fits u32")
                } else {
                    8
                },
                20,
                100,
            )
            .expect("model budget"),
            AgentPolicyInstant::from_millis(NOW),
        );
        let model = AgentProviderModelRevision::try_new(
            match provider {
                AgentProviderKind::OpenAiResponses => "gpt-5.6-terra",
                AgentProviderKind::AnthropicMessages => "claude-opus-5",
            }
            .to_owned(),
        )
        .expect("model");
        let reasoning = match provider {
            AgentProviderKind::OpenAiResponses => AgentProviderReasoningEffort::Medium,
            AgentProviderKind::AnthropicMessages => AgentProviderReasoningEffort::None,
        };
        let schedule =
            fixture_pricing_schedule(provider, model, reasoning, tokenizer, pricing_profile)
                .expect("provider schedule");
        let config = if let Some(model) = chosen {
            model.try_call_config(20).expect("chosen model config")
        } else if provider_exact {
            schedule
                .try_provider_exact_call_config(20, AgentProviderStreamBudget::STANDARD)
                .expect("provider-exact config")
        } else {
            schedule
                .try_call_config(5, 20, AgentProviderStreamBudget::STANDARD)
                .expect("provider config")
        };
        let prepared = match provider {
            AgentProviderKind::OpenAiResponses if provider_exact => {
                AgentPreparedObservationRequest::try_openai_for_provider_exact_count(
                    &mut policy,
                    request,
                    &observation,
                    payload,
                    &objective,
                    config.clone(),
                )
            }
            AgentProviderKind::OpenAiResponses => AgentPreparedObservationRequest::try_openai(
                &mut policy,
                request,
                &observation,
                payload,
                &objective,
                config.clone(),
            ),
            AgentProviderKind::AnthropicMessages => AgentPreparedObservationRequest::try_anthropic(
                &mut policy,
                request,
                &observation,
                payload,
                &objective,
                config.clone(),
            ),
        }
        .expect("prepared request");
        StatefulProviderFixture {
            policy,
            input: prepared.into_transport_input(),
            observation,
            account,
            lease,
            config,
        }
    }

    fn successor_observation(previous: &SemanticObservation) -> SemanticObservation {
        let invocation = SemanticInvocationId::new(12).expect("successor invocation");
        let wire = serde_json::to_vec(&json!({
            "v": SEMANTIC_WIRE_VERSION,
            "i": invocation.get(),
            "g": 12,
            "c": "complete",
            "n": [
                {"k": 1, "r": "document", "o": 16},
                {"k": 2, "p": 0, "r": "paragraph", "t": "updated synthetic marker"}
            ]
        }))
        .expect("successor wire");
        let snapshot = decode_semantic_snapshot(
            SemanticDecodeContext::new(
                invocation,
                previous.frames()[0].frame().clone(),
                SemanticSnapshotGeneration::new(12).expect("successor snapshot generation"),
            ),
            &wire,
        )
        .expect("successor snapshot");
        SemanticObservationAssembler::new(
            SemanticObservationRequest::initial(
                SemanticObservationId::new(2).expect("successor observation"),
                previous.request().context(),
                SemanticObservationBudget::try_new(8, 4_096, 1)
                    .expect("successor observation budget"),
            ),
            snapshot,
        )
        .expect("successor observation assembler")
        .finish()
        .expect("successor observation")
    }

    fn pricing_schedule(config: &AgentProviderCallConfig) -> AgentProviderPricingSchedule {
        fixture_pricing_schedule(
            config.provider(),
            config.model().clone(),
            config.reasoning_effort(),
            config.tokenizer().clone(),
            config.pricing_profile(),
        )
        .expect("pricing schedule")
    }

    fn fixture_pricing_schedule(
        provider: AgentProviderKind,
        model: AgentProviderModelRevision,
        reasoning: AgentProviderReasoningEffort,
        tokenizer: SemanticTokenizerRevision,
        profile: AgentProviderPricingProfile,
    ) -> Result<AgentProviderPricingSchedule, AgentProviderPricingContractError> {
        let rates = match provider {
            AgentProviderKind::OpenAiResponses => {
                AgentProviderTokenRates::try_new(1_000_000, 500_000, 1_250_000, 21_000_000)
                    .expect("OpenAI synthetic rates")
            }
            AgentProviderKind::AnthropicMessages => {
                AgentProviderTokenRates::try_new(1_000_000, 1_000_000, 1_000_000, 10_000_000)
                    .expect("Anthropic synthetic rates")
            }
        };
        let route = match provider {
            AgentProviderKind::OpenAiResponses => AgentProviderResponseRoute::OpenAiDefault,
            AgentProviderKind::AnthropicMessages => {
                AgentProviderResponseRoute::AnthropicStandardGlobal
            }
        };
        AgentProviderPricingSchedule::try_new(
            provider,
            model.clone(),
            vec![model],
            route,
            reasoning,
            tokenizer,
            profile,
            rates,
        )
    }

    struct CapturedRequest {
        head: Vec<u8>,
        body: Vec<u8>,
    }

    struct OneShotServer {
        openai: Url,
        anthropic: Url,
        request: Receiver<Result<CapturedRequest, &'static str>>,
        thread: Option<JoinHandle<()>>,
    }

    struct ScriptedResponse {
        delay: Duration,
        content_type: &'static str,
        body: Vec<u8>,
    }

    struct SequenceServer {
        openai: Url,
        anthropic: Url,
        requests: Receiver<Result<Vec<CapturedRequest>, &'static str>>,
        thread: Option<JoinHandle<()>>,
    }

    struct DropConnectionServer {
        openai: Url,
        anthropic: Url,
        requests: Receiver<usize>,
        thread: Option<JoinHandle<()>>,
    }

    struct StalledResponseServer {
        openai: Url,
        anthropic: Url,
        ready: Receiver<()>,
        release: SyncSender<()>,
        request: Receiver<Result<CapturedRequest, &'static str>>,
        thread: Option<JoinHandle<()>>,
    }

    struct HeldEofServer {
        openai: Url,
        anthropic: Url,
        terminal_sent: Receiver<()>,
        release: SyncSender<()>,
        request: Receiver<Result<CapturedRequest, &'static str>>,
        thread: Option<JoinHandle<()>>,
    }

    impl HeldEofServer {
        fn spawn(body: Vec<u8>) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
            let address = listener.local_addr().expect("loopback address");
            let openai = Url::parse(&format!("http://{address}/v1/responses")).expect("openai URL");
            let anthropic =
                Url::parse(&format!("http://{address}/v1/messages")).expect("anthropic URL");
            let (terminal_sender, terminal_sent) = mpsc::sync_channel(1);
            let (release, release_receiver) = mpsc::sync_channel(1);
            let (request_sender, request) = mpsc::sync_channel(1);
            let thread = thread::spawn(move || {
                let result = listener.accept().map_err(|_| "accept failed").and_then(
                    |(mut stream, _)| {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .map_err(|_| "read deadline failed")?;
                        let captured = read_request(&mut stream)?;
                        stream
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Encoding: identity\r\nConnection: close\r\n\r\n",
                            )
                            .map_err(|_| "response head failed")?;
                        stream
                            .write_all(&body)
                            .map_err(|_| "response body failed")?;
                        stream.flush().map_err(|_| "response flush failed")?;
                        terminal_sender.send(()).map_err(|_| "signal failed")?;
                        release_receiver
                            .recv_timeout(Duration::from_secs(3))
                            .map_err(|_| "release failed")?;
                        Ok(captured)
                    },
                );
                let _sent = request_sender.send(result);
            });
            Self {
                openai,
                anthropic,
                terminal_sent,
                release,
                request,
                thread: Some(thread),
            }
        }

        fn wait_until_terminal_sent(&self) {
            self.terminal_sent
                .recv_timeout(Duration::from_secs(3))
                .expect("terminal bytes sent");
        }

        fn terminal_observed(&self) -> bool {
            match self.terminal_sent.try_recv() {
                Ok(()) => true,
                Err(mpsc::TryRecvError::Empty) => false,
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("terminal-observation server disconnected")
                }
            }
        }

        fn release_eof(&self) {
            self.release.send(()).expect("release EOF");
        }

        fn finish(mut self) -> CapturedRequest {
            let request = self
                .request
                .recv_timeout(Duration::from_secs(3))
                .expect("server result")
                .expect("valid request");
            self.thread
                .take()
                .expect("server thread")
                .join()
                .expect("server join");
            request
        }
    }

    impl StalledResponseServer {
        fn spawn() -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
            let address = listener.local_addr().expect("loopback address");
            let openai = Url::parse(&format!("http://{address}/v1/responses")).expect("openai URL");
            let anthropic =
                Url::parse(&format!("http://{address}/v1/messages")).expect("anthropic URL");
            let (ready_sender, ready) = mpsc::sync_channel(1);
            let (release, release_receiver) = mpsc::sync_channel(1);
            let (request_sender, request) = mpsc::sync_channel(1);
            let thread =
                thread::spawn(move || {
                    let result = listener.accept().map_err(|_| "accept failed").and_then(
                        |(mut stream, _)| {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(3)))
                                .map_err(|_| "read deadline failed")?;
                            let captured = read_request(&mut stream)?;
                            ready_sender.send(()).map_err(|_| "ready failed")?;
                            release_receiver
                                .recv_timeout(Duration::from_secs(3))
                                .map_err(|_| "release failed")?;
                            Ok(captured)
                        },
                    );
                    let _sent = request_sender.send(result);
                });
            Self {
                openai,
                anthropic,
                ready,
                release,
                request,
                thread: Some(thread),
            }
        }

        fn wait_until_request(&self) {
            self.ready
                .recv_timeout(Duration::from_secs(3))
                .expect("request dispatch");
        }

        fn request_observed(&self) -> bool {
            match self.ready.try_recv() {
                Ok(()) => true,
                Err(mpsc::TryRecvError::Empty) => false,
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("request-observation server disconnected")
                }
            }
        }

        fn finish(mut self) -> CapturedRequest {
            self.release.send(()).expect("release server");
            let request = self
                .request
                .recv_timeout(Duration::from_secs(3))
                .expect("server result")
                .expect("valid request");
            self.thread
                .take()
                .expect("server thread")
                .join()
                .expect("server join");
            request
        }
    }

    impl DropConnectionServer {
        fn spawn() -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
            listener.set_nonblocking(true).expect("nonblocking");
            let address = listener.local_addr().expect("loopback address");
            let openai = Url::parse(&format!("http://{address}/v1/responses")).expect("openai URL");
            let anthropic =
                Url::parse(&format!("http://{address}/v1/messages")).expect("anthropic URL");
            let (sender, requests) = mpsc::sync_channel(1);
            let thread = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(500);
                let mut count = 0;
                while Instant::now() < deadline {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // Count transport attempts at accept. This fixture
                            // intentionally drops the connection, so request
                            // parsing is not a prerequisite for proving that
                            // the HTTP client opened another attempt.
                            count += 1;
                            let _deadline = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let _request = read_request(&mut stream);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
                let _sent = sender.send(count);
            });
            Self {
                openai,
                anthropic,
                requests,
                thread: Some(thread),
            }
        }

        fn finish(mut self) -> usize {
            let requests = self
                .requests
                .recv_timeout(Duration::from_secs(2))
                .expect("request count");
            self.thread
                .take()
                .expect("server thread")
                .join()
                .expect("server join");
            requests
        }
    }

    impl OneShotServer {
        fn spawn(status: &str, response_headers: &[(&str, &str)], chunks: Vec<Vec<u8>>) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
            let address = listener.local_addr().expect("loopback address");
            let openai = Url::parse(&format!("http://{address}/v1/responses")).expect("openai URL");
            let anthropic =
                Url::parse(&format!("http://{address}/v1/messages")).expect("anthropic URL");
            let body_len = chunks.iter().map(Vec::len).sum::<usize>();
            let mut response_head =
                format!("HTTP/1.1 {status}\r\nContent-Length: {body_len}\r\nConnection: close\r\n");
            for (name, value) in response_headers {
                response_head.push_str(name);
                response_head.push_str(": ");
                response_head.push_str(value);
                response_head.push_str("\r\n");
            }
            response_head.push_str("\r\n");
            let response_head = response_head.into_bytes();
            let (sender, request) = mpsc::sync_channel(1);
            let thread =
                thread::spawn(move || {
                    let result = listener.accept().map_err(|_| "accept failed").and_then(
                        |(mut stream, _)| {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(3)))
                                .map_err(|_| "read deadline failed")?;
                            let captured = read_request(&mut stream)?;
                            stream
                                .write_all(&response_head)
                                .map_err(|_| "response head failed")?;
                            for chunk in chunks {
                                stream
                                    .write_all(&chunk)
                                    .map_err(|_| "response body failed")?;
                                thread::sleep(Duration::from_millis(5));
                            }
                            Ok(captured)
                        },
                    );
                    let _sent = sender.send(result);
                });
            Self {
                openai,
                anthropic,
                request,
                thread: Some(thread),
            }
        }

        fn finish(mut self) -> CapturedRequest {
            let request = self
                .request
                .recv_timeout(Duration::from_secs(5))
                .expect("server result")
                .expect("valid request");
            self.thread
                .take()
                .expect("server thread")
                .join()
                .expect("server join");
            request
        }
    }

    impl SequenceServer {
        fn spawn(scripts: Vec<ScriptedResponse>) -> Self {
            Self::spawn_with_statuses(
                scripts
                    .into_iter()
                    .map(|script| ("200 OK", script))
                    .collect(),
            )
        }

        fn spawn_with_statuses(scripts: Vec<(&'static str, ScriptedResponse)>) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
            let address = listener.local_addr().expect("loopback address");
            let openai = Url::parse(&format!("http://{address}/v1/responses")).expect("openai URL");
            let anthropic =
                Url::parse(&format!("http://{address}/v1/messages")).expect("anthropic URL");
            let (sender, requests) = mpsc::sync_channel(1);
            let thread = thread::spawn(move || {
                let mut captured = Vec::with_capacity(scripts.len());
                let result = (|| {
                    for (status, script) in scripts {
                        let (mut stream, _) = listener.accept().map_err(|_| "accept failed")?;
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .map_err(|_| "read deadline failed")?;
                        captured.push(read_request(&mut stream)?);
                        thread::sleep(script.delay);
                        let head = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {}\r\nContent-Encoding: identity\r\nConnection: close\r\n\r\n",
                            script.body.len(),
                            script.content_type,
                        );
                        let _head_result = stream.write_all(head.as_bytes());
                        let _body_result = stream.write_all(&script.body);
                    }
                    Ok(captured)
                })();
                let _sent = sender.send(result);
            });
            Self {
                openai,
                anthropic,
                requests,
                thread: Some(thread),
            }
        }

        fn finish(mut self) -> Vec<CapturedRequest> {
            let requests = self
                .requests
                .recv_timeout(Duration::from_secs(5))
                .expect("server result")
                .expect("valid requests");
            self.thread
                .take()
                .expect("server thread")
                .join()
                .expect("server join");
            requests
        }
    }

    fn read_request(stream: &mut TcpStream) -> Result<CapturedRequest, &'static str> {
        let mut bytes = Vec::with_capacity(8 * 1_024);
        let mut buffer = [0_u8; 8 * 1_024];
        let (header_end, content_length) = loop {
            let read = stream
                .read(&mut buffer)
                .map_err(|_| "request read failed")?;
            if read == 0 {
                return Err("request ended before headers");
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.len() > MAX_AGENT_PROVIDER_REQUEST_BYTES + 64 * 1_024 {
                return Err("request exceeded fixture bound");
            }
            if let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") {
                let head = std::str::from_utf8(&bytes[..header_end])
                    .map_err(|_| "request headers were not ASCII")?;
                let content_length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .ok_or("content length missing")?;
                break (header_end + 4, content_length);
            }
        };
        while bytes.len() < header_end.saturating_add(content_length) {
            let read = stream
                .read(&mut buffer)
                .map_err(|_| "request body failed")?;
            if read == 0 {
                return Err("request body ended early");
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        if bytes.len() != header_end.saturating_add(content_length) {
            return Err("request body length changed");
        }
        Ok(CapturedRequest {
            head: bytes[..header_end].to_vec(),
            body: bytes[header_end..].to_vec(),
        })
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn test_transport(server: &OneShotServer) -> AgentProviderTransport {
        AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::try_new(
                Duration::from_secs(5),
                Duration::from_secs(2),
                Duration::from_secs(2),
            )
            .expect("config"),
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport")
    }

    fn openai_success_stream() -> Vec<u8> {
        [
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"status\":\"in_progress\",\"model\":\"gpt-5.6-terra\",\"service_tier\":\"default\"}}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"status\":\"in_progress\",\"role\":\"assistant\"}}\n\n",
            "event: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\"}}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"hello\"}\n\n",
            "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"text\":\"hello\"}\n\n",
            "event: response.content_part.done\ndata: {\"type\":\"response.content_part.done\",\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\"}}\n\n",
            "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_1\",\"status\":\"completed\",\"role\":\"assistant\"}}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"model\":\"gpt-5.6-terra\",\"service_tier\":\"default\",\"output\":[{\"type\":\"message\",\"id\":\"msg_1\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\"}]}],\"usage\":{\"input_tokens\":17,\"output_tokens\":3,\"total_tokens\":20,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens_details\":{\"reasoning_tokens\":0}}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .concat()
        .into_bytes()
    }

    fn openai_tool_stream() -> Vec<u8> {
        [
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_tool_1\",\"status\":\"in_progress\",\"model\":\"gpt-5.6-terra\",\"service_tier\":\"default\"}}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_tool_1\",\"call_id\":\"call_tool_1\",\"name\":\"back\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\n",
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_tool_1\",\"delta\":\"{}\"}\n\n",
            "event: response.function_call_arguments.done\ndata: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_tool_1\",\"name\":\"back\",\"arguments\":\"{}\"}\n\n",
            "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_tool_1\",\"call_id\":\"call_tool_1\",\"name\":\"back\",\"arguments\":\"{}\",\"status\":\"completed\"}}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_tool_1\",\"status\":\"completed\",\"model\":\"gpt-5.6-terra\",\"service_tier\":\"default\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_tool_1\",\"call_id\":\"call_tool_1\",\"name\":\"back\",\"arguments\":\"{}\",\"status\":\"completed\"}],\"usage\":{\"input_tokens\":17,\"output_tokens\":3,\"total_tokens\":20,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens_details\":{\"reasoning_tokens\":0}}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .concat()
        .into_bytes()
    }

    fn anthropic_tool_stream() -> Vec<u8> {
        [
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_tool_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-opus-5\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":7,\"cache_creation_input_tokens\":3,\"cache_read_input_tokens\":5,\"output_tokens\":1,\"output_tokens_details\":{\"thinking_tokens\":0},\"service_tier\":\"standard\",\"inference_geo\":\"global\"}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_back_1\",\"name\":\"back\",\"input\":{}}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":4}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ]
        .concat()
        .into_bytes()
    }

    fn provider_tool_stream(provider: AgentProviderKind) -> Vec<u8> {
        match provider {
            AgentProviderKind::OpenAiResponses => openai_tool_stream(),
            AgentProviderKind::AnthropicMessages => anthropic_tool_stream(),
        }
    }

    fn sse_json(event: &str, payload: serde_json::Value) -> String {
        format!(
            "event: {event}\ndata: {}\n\n",
            serde_json::to_string(&payload).expect("SSE fixture JSON")
        )
    }

    fn openai_extract_tool_stream() -> Vec<u8> {
        let arguments = r#"{"schema_id":71}"#;
        [
            sse_json(
                "response.created",
                json!({"type":"response.created","response":{
                    "id":"resp_extract_tool_1","status":"in_progress",
                    "model":"gpt-5.6-terra","service_tier":"default"
                }}),
            ),
            sse_json(
                "response.output_item.added",
                json!({"type":"response.output_item.added","output_index":0,"item":{
                    "type":"function_call","id":"fc_extract_tool_1",
                    "call_id":"call_extract_tool_1","name":"extract",
                    "arguments":"","status":"in_progress"
                }}),
            ),
            sse_json(
                "response.function_call_arguments.delta",
                json!({"type":"response.function_call_arguments.delta",
                    "item_id":"fc_extract_tool_1","delta":arguments}),
            ),
            sse_json(
                "response.function_call_arguments.done",
                json!({"type":"response.function_call_arguments.done",
                    "item_id":"fc_extract_tool_1","name":"extract",
                    "arguments":arguments}),
            ),
            sse_json(
                "response.output_item.done",
                json!({"type":"response.output_item.done","output_index":0,"item":{
                    "type":"function_call","id":"fc_extract_tool_1",
                    "call_id":"call_extract_tool_1","name":"extract",
                    "arguments":arguments,"status":"completed"
                }}),
            ),
            sse_json(
                "response.completed",
                json!({"type":"response.completed","response":{
                    "id":"resp_extract_tool_1","status":"completed",
                    "model":"gpt-5.6-terra","service_tier":"default",
                    "output":[{"type":"function_call","id":"fc_extract_tool_1",
                        "call_id":"call_extract_tool_1","name":"extract",
                        "arguments":arguments,"status":"completed"}],
                    "usage":{"input_tokens":17,"output_tokens":3,"total_tokens":20,
                        "input_tokens_details":{"cached_tokens":0},
                        "output_tokens_details":{"reasoning_tokens":0}}
                }}),
            ),
            "data: [DONE]\n\n".to_owned(),
        ]
        .concat()
        .into_bytes()
    }

    fn anthropic_extract_tool_stream() -> Vec<u8> {
        [
            sse_json(
                "message_start",
                json!({"type":"message_start","message":{
                    "id":"msg_extract_tool_1","type":"message","role":"assistant",
                    "content":[],"model":"claude-opus-5","stop_reason":null,
                    "stop_sequence":null,"usage":{"input_tokens":7,
                        "cache_creation_input_tokens":3,"cache_read_input_tokens":5,
                        "output_tokens":1,"output_tokens_details":{"thinking_tokens":0},
                        "service_tier":"standard","inference_geo":"global"}
                }}),
            ),
            sse_json(
                "content_block_start",
                json!({"type":"content_block_start","index":0,"content_block":{
                    "type":"tool_use","id":"toolu_extract_1","name":"extract",
                    "input":{}
                }}),
            ),
            sse_json(
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,"delta":{
                    "type":"input_json_delta","partial_json":"{\"schema_id\":71}"
                }}),
            ),
            sse_json(
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
            ),
            sse_json(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use",
                    "stop_sequence":null},"usage":{"output_tokens":4}}),
            ),
            sse_json("message_stop", json!({"type":"message_stop"})),
        ]
        .concat()
        .into_bytes()
    }

    fn provider_extract_tool_stream(provider: AgentProviderKind) -> Vec<u8> {
        match provider {
            AgentProviderKind::OpenAiResponses => openai_extract_tool_stream(),
            AgentProviderKind::AnthropicMessages => anthropic_extract_tool_stream(),
        }
    }

    fn openai_extraction_output_stream(output: &str, input_tokens: u32) -> Vec<u8> {
        [
            sse_json(
                "response.created",
                json!({"type":"response.created","response":{
                    "id":"resp_extract_output_1","status":"in_progress",
                    "model":"gpt-5.6-terra","service_tier":"default"
                }}),
            ),
            sse_json(
                "response.output_item.added",
                json!({"type":"response.output_item.added","output_index":0,"item":{
                    "type":"message","id":"msg_extract_output_1",
                    "status":"in_progress","role":"assistant"
                }}),
            ),
            sse_json(
                "response.content_part.added",
                json!({"type":"response.content_part.added",
                    "item_id":"msg_extract_output_1","output_index":0,"content_index":0,
                    "part":{"type":"output_text"}}),
            ),
            sse_json(
                "response.output_text.delta",
                json!({"type":"response.output_text.delta",
                    "item_id":"msg_extract_output_1","output_index":0,"content_index":0,
                    "delta":output}),
            ),
            sse_json(
                "response.output_text.done",
                json!({"type":"response.output_text.done",
                    "item_id":"msg_extract_output_1","output_index":0,"content_index":0,
                    "text":output}),
            ),
            sse_json(
                "response.content_part.done",
                json!({"type":"response.content_part.done",
                    "item_id":"msg_extract_output_1","output_index":0,"content_index":0,
                    "part":{"type":"output_text"}}),
            ),
            sse_json(
                "response.output_item.done",
                json!({"type":"response.output_item.done","output_index":0,"item":{
                    "type":"message","id":"msg_extract_output_1",
                    "status":"completed","role":"assistant"
                }}),
            ),
            sse_json(
                "response.completed",
                json!({"type":"response.completed","response":{
                    "id":"resp_extract_output_1","status":"completed",
                    "model":"gpt-5.6-terra","service_tier":"default",
                    "output":[{"type":"message","id":"msg_extract_output_1",
                        "status":"completed","role":"assistant",
                        "content":[{"type":"output_text"}]}],
                    "usage":{"input_tokens":input_tokens,"output_tokens":3,
                        "total_tokens":input_tokens + 3,
                        "input_tokens_details":{"cached_tokens":0},
                        "output_tokens_details":{"reasoning_tokens":0}}
                }}),
            ),
            "data: [DONE]\n\n".to_owned(),
        ]
        .concat()
        .into_bytes()
    }

    fn anthropic_extraction_output_stream(output: &str, input_tokens: u32) -> Vec<u8> {
        [
            sse_json(
                "message_start",
                json!({"type":"message_start","message":{
                    "id":"msg_extract_output_1","type":"message","role":"assistant",
                    "content":[],"model":"claude-opus-5","stop_reason":null,
                    "stop_sequence":null,"usage":{"input_tokens":input_tokens,
                        "cache_creation_input_tokens":0,"cache_read_input_tokens":0,
                        "output_tokens":1,"output_tokens_details":{"thinking_tokens":0},
                        "service_tier":"standard","inference_geo":"global"}
                }}),
            ),
            sse_json(
                "content_block_start",
                json!({"type":"content_block_start","index":0,
                    "content_block":{"type":"text","text":""}}),
            ),
            sse_json(
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,
                    "delta":{"type":"text_delta","text":output}}),
            ),
            sse_json(
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
            ),
            sse_json(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn",
                    "stop_sequence":null},"usage":{"output_tokens":4}}),
            ),
            sse_json("message_stop", json!({"type":"message_stop"})),
        ]
        .concat()
        .into_bytes()
    }

    fn provider_extraction_output_stream(
        provider: AgentProviderKind,
        output: &str,
        input_tokens: u32,
    ) -> Vec<u8> {
        match provider {
            AgentProviderKind::OpenAiResponses => {
                openai_extraction_output_stream(output, input_tokens)
            }
            AgentProviderKind::AnthropicMessages => {
                anthropic_extraction_output_stream(output, input_tokens)
            }
        }
    }

    fn openai_mixed_tool_stream() -> Vec<u8> {
        [
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_mixed_1\",\"status\":\"in_progress\",\"model\":\"gpt-5.6-terra\",\"service_tier\":\"default\"}}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_mixed_1\",\"status\":\"in_progress\",\"role\":\"assistant\"}}\n\n",
            "event: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"item_id\":\"msg_mixed_1\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\"}}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_mixed_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"working\"}\n\n",
            "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"item_id\":\"msg_mixed_1\",\"output_index\":0,\"content_index\":0,\"text\":\"working\"}\n\n",
            "event: response.content_part.done\ndata: {\"type\":\"response.content_part.done\",\"item_id\":\"msg_mixed_1\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\"}}\n\n",
            "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_mixed_1\",\"status\":\"completed\",\"role\":\"assistant\"}}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"id\":\"fc_mixed_1\",\"call_id\":\"call_mixed_1\",\"name\":\"back\",\"arguments\":\"\",\"status\":\"in_progress\"}}\n\n",
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_mixed_1\",\"delta\":\"{}\"}\n\n",
            "event: response.function_call_arguments.done\ndata: {\"type\":\"response.function_call_arguments.done\",\"item_id\":\"fc_mixed_1\",\"name\":\"back\",\"arguments\":\"{}\"}\n\n",
            "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"id\":\"fc_mixed_1\",\"call_id\":\"call_mixed_1\",\"name\":\"back\",\"arguments\":\"{}\",\"status\":\"completed\"}}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_mixed_1\",\"status\":\"completed\",\"model\":\"gpt-5.6-terra\",\"service_tier\":\"default\",\"output\":[{\"type\":\"message\",\"id\":\"msg_mixed_1\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\"}]},{\"type\":\"function_call\",\"id\":\"fc_mixed_1\",\"call_id\":\"call_mixed_1\",\"name\":\"back\",\"arguments\":\"{}\",\"status\":\"completed\"}],\"usage\":{\"input_tokens\":17,\"output_tokens\":3,\"total_tokens\":20,\"input_tokens_details\":{\"cached_tokens\":0},\"output_tokens_details\":{\"reasoning_tokens\":0}}}}\n\n",
            "data: [DONE]\n\n",
        ]
        .concat()
        .into_bytes()
    }

    fn anthropic_failure_stream() -> Vec<u8> {
        [
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-opus-5\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":7,\"cache_creation_input_tokens\":3,\"cache_read_input_tokens\":5,\"output_tokens\":1,\"output_tokens_details\":{\"thinking_tokens\":0},\"service_tier\":\"standard\",\"inference_geo\":\"global\"}}}\n\n",
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"provider-authored-sensitive-detail\"}}\n\n",
        ]
        .concat()
        .into_bytes()
    }

    fn anthropic_failure_without_usage_stream() -> Vec<u8> {
        "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"provider-authored-sensitive-detail\"}}\n\n"
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn configuration_credentials_and_response_headers_are_strict_and_redacted() {
        assert_eq!(
            AgentProviderTransportConfig::try_new(
                Duration::ZERO,
                Duration::from_secs(1),
                Duration::from_secs(1)
            ),
            Err(AgentProviderTransportConfigError::Deadlines)
        );
        assert_eq!(
            AgentProviderTransportConfig::try_new(
                Duration::from_secs(5),
                Duration::from_secs(6),
                Duration::from_secs(1)
            ),
            Err(AgentProviderTransportConfigError::Deadlines)
        );
        for invalid in ["", "has space", "line\nbreak"] {
            assert_eq!(
                AgentProviderCredential::try_new(
                    AgentProviderKind::OpenAiResponses,
                    invalid.to_owned()
                )
                .expect_err("invalid credential"),
                AgentProviderCredentialError::Content
            );
        }
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let debug = format!("{credential:?}");
        assert!(!debug.contains("synthetic-openai-key"));
        let header = sensitive_header(credential.provider, &credential.secret).expect("header");
        assert!(header.is_sensitive());
        assert!(!format!("{header:?}").contains("synthetic-openai-key"));
        let (_, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let call = input.request().call();

        let request = provider_request(
            &Client::new(),
            Url::parse(OPENAI_RESPONSES_URL).expect("fixed endpoint"),
            credential.provider,
            call,
            header,
            br#"{"input":"private request body marker"}"#.to_vec(),
        )
        .expect("client request identity")
        .build()
        .expect("fixed request");
        let request_id = request
            .headers()
            .get(OPENAI_CLIENT_REQUEST_ID_HEADER)
            .expect("client request id");
        assert_eq!(request_id.as_bytes().len(), 54);
        assert_ne!(
            request_id,
            &openai_client_request_id(call, ProviderRequestPhase::InputTokens)
                .expect("input-token request id")
        );
        let request_debug = format!("{request:?}");
        assert!(!request_debug.contains("synthetic-openai-key"));
        assert!(!request_debug.contains("private request body marker"));

        let production = ProviderEndpoints::production().expect("production endpoints");
        assert!(production.https_only);
        assert_eq!(production.openai.as_str(), OPENAI_RESPONSES_URL);
        assert_eq!(production.anthropic.as_str(), ANTHROPIC_MESSAGES_URL);

        let loopback_openai = Url::parse("http://127.0.0.1:43123/custom/responses-v1_0~probe")
            .expect("loopback OpenAI");
        let loopback_anthropic =
            Url::parse("http://127.0.0.1:43123/custom/messages").expect("loopback Anthropic");
        assert!(exact_loopback_url(loopback_openai.as_str()));
        assert!(exact_loopback_url(loopback_anthropic.as_str()));
        let loopback =
            ProviderEndpoints::loopback(loopback_openai.as_str(), loopback_anthropic.as_str())
                .expect("exact diagnostic endpoints");
        assert!(!loopback.https_only);
        assert_eq!(loopback.openai, loopback_openai);
        assert_eq!(loopback.anthropic, loopback_anthropic);
        assert_eq!(
            loopback.openai_input_tokens.as_str(),
            "http://127.0.0.1:43123/custom/responses-v1_0~probe/input_tokens"
        );

        for ambiguous_path in [
            "/v1/%2e/responses",
            "/v1/%2E/responses",
            "/v1/%2f/responses",
            "/v1/%2F/responses",
            "/v1/%5c/responses",
            "/v1/%5C/responses",
            "/v1/%00/responses",
            "/v1/%1f/responses",
            "/v1/%20/responses",
            "/v1/./responses",
            "/v1/../responses",
            "/v1//responses",
            "/v1/responses/",
        ] {
            assert!(
                !exact_loopback_path(ambiguous_path),
                "accepted ambiguous path {ambiguous_path}"
            );
        }

        for invalid in [
            "http://127.0.0.1/response",
            "http://127.0.0.1:80/response",
            "http://127.0.0.1:0/response",
            "https://127.0.0.1:43123/response",
            "http://localhost:43123/response",
            "http://[::1]:43123/response",
            "http://user@127.0.0.1:43123/response",
            "http://user:pass@127.0.0.1:43123/response",
            "http://127.0.0.1:43123",
            "http://127.0.0.1:43123//",
            "http://127.0.0.1:43123/v1//responses",
            "http://127.0.0.1:43123/v1/responses/",
            "http://127.0.0.1:43123/v1/%2f/responses",
            "http://127.0.0.1:43123/v1/%2F/responses",
            "http://127.0.0.1:43123/v1/%5c/responses",
            "http://127.0.0.1:43123/v1/%5C/responses",
            "http://127.0.0.1:43123/v1/%00/responses",
            "http://127.0.0.1:43123/v1/%1f/responses",
            "http://127.0.0.1:43123/v1/%20/responses",
            "http://127.0.0.1:43123/v1/./responses",
            "http://127.0.0.1:43123/v1/../responses",
            "http://127.0.0.1:43123/?count=other",
            "http://127.0.0.1:43123/response?count=other",
            "http://127.0.0.1:43123/response#fragment",
        ] {
            assert!(!exact_loopback_url(invalid), "accepted {invalid}");
            assert_eq!(
                ProviderEndpoints::loopback(invalid, loopback_anthropic.as_str()).err(),
                Some(AgentProviderTransportConfigError::Endpoint)
            );
        }

        // `url::Url` would erase these spellings via the WHATWG dot-segment
        // algorithm. Admission intentionally takes the source string and
        // rejects it before that normalization can make it indistinguishable
        // from the permitted canonical route.
        for normalized in [
            "http://127.0.0.1:43123/v1/./responses",
            "http://127.0.0.1:43123/v1/%2e/responses",
            "http://127.0.0.1:43123/v1/%2E/responses",
            "http://127.0.0.1:43123/v1/segment/../responses",
        ] {
            let parsed = Url::parse(normalized).expect("normalizable loopback URL");
            assert_eq!(parsed.path(), "/v1/responses");
            assert_ne!(parsed.as_str(), normalized);
            assert!(!exact_loopback_url(normalized));
        }

        let canonical_openai = "http://127.0.0.1:43123/v1/responses";
        let canonical = ProviderEndpoints::loopback(canonical_openai, loopback_anthropic.as_str())
            .expect("canonical loopback URL");
        assert_eq!(
            canonical.openai_input_tokens.as_str(),
            "http://127.0.0.1:43123/v1/responses/input_tokens"
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        assert!(response_content_type_admitted(&headers));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        assert!(!response_content_type_admitted(&headers));
        assert!(response_json_content_type_admitted(&headers));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        assert!(response_json_content_type_admitted(&headers));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        assert!(!response_json_content_type_admitted(&headers));
        headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(
            parse_retry_after(&headers).map(AgentProviderRetryAfter::millis),
            Some(2_000)
        );
        headers.insert(RETRY_AFTER, HeaderValue::from_static("02"));
        assert_eq!(parse_retry_after(&headers), None);

        let header_limit = usize::try_from(MAX_AGENT_PROVIDER_RESPONSE_HEADER_BYTES)
            .expect("header limit fits host usize");
        let field_name = HeaderName::from_static("x-agent-boundary");
        let exact_value_len = header_limit
            .checked_sub(field_name.as_str().len())
            .and_then(|remaining| remaining.checked_sub(HTTP_HEADER_FIELD_OVERHEAD_BYTES))
            .expect("header limit exceeds field overhead");
        let exact_value = vec![b'a'; exact_value_len];
        let mut bounded_headers = HeaderMap::new();
        bounded_headers.insert(
            field_name.clone(),
            HeaderValue::from_bytes(&exact_value).expect("bounded header value"),
        );
        assert!(response_headers_admitted(&bounded_headers));
        bounded_headers.append(field_name, HeaderValue::from_static("b"));
        assert!(!response_headers_admitted(&bounded_headers));

        let mut declared_body = HeaderMap::new();
        assert!(response_content_length_admitted(&declared_body, 1_024));
        declared_body.insert(CONTENT_LENGTH, HeaderValue::from_static("1024"));
        assert!(response_content_length_admitted(&declared_body, 1_024));
        declared_body.insert(CONTENT_LENGTH, HeaderValue::from_static("1025"));
        assert!(!response_content_length_admitted(&declared_body, 1_024));
        for invalid in ["", "00", "01", "+1", "1_024", "18446744073709551616"] {
            declared_body.insert(
                CONTENT_LENGTH,
                HeaderValue::from_str(invalid).expect("syntactically valid header value"),
            );
            assert!(!response_content_length_admitted(&declared_body, 1_024));
        }
        declared_body.insert(CONTENT_LENGTH, HeaderValue::from_static("1"));
        declared_body.append(CONTENT_LENGTH, HeaderValue::from_static("1"));
        assert!(!response_content_length_admitted(&declared_body, 1_024));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_input_count_is_exact_request_bound_and_separate_from_model_dispatch() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "application/json"),
                ("Content-Encoding", "identity"),
            ],
            vec![br#"{"object":"response.input_tokens","input_tokens":17}"#.to_vec()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_exact_fixture();
        let mut input_reducer = provider_input_metrics(&policy, 41_001);
        let reserved_before_count = policy.accounting().reserved_model_tokens();
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        assert_eq!(
            attempt.input_metric_receipt(),
            None,
            "provisional conservative accounting must not be recordable"
        );
        let admitted_call = attempt.call().expect("live call");
        let admitted_metrics = attempt.input_metrics().expect("live input metrics");
        assert_eq!(
            admitted_metrics
                .semantic_payload_tokens()
                .expect("semantic reservation")
                .quality(),
            SemanticTokenCountQuality::Conservative
        );
        let conservative_structured = admitted_metrics
            .structured_input_tokens()
            .expect("whole-request reservation");
        assert_eq!(
            conservative_structured.tokens(),
            admitted_metrics.serialized_request_bytes()
        );
        assert_eq!(
            conservative_structured.quality(),
            SemanticTokenCountQuality::Conservative
        );
        let counted = match attempt.count_openai_input_tokens().await {
            AgentProviderExactCountOutcome::Counted(counted) => counted,
            AgentProviderExactCountOutcome::Failed(_) => panic!("exact count expected"),
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("count drive unavailable: {error:?}")
            }
        };
        let counted_metric_receipt = counted.input_metric_receipt();
        assert_eq!(counted_metric_receipt.call(), admitted_call.call());
        assert_eq!(counted_metric_receipt.metrics(), counted.input_metrics());
        input_reducer
            .record(counted_metric_receipt)
            .expect("post-count ProviderExact metrics are recordable");
        assert_eq!(input_reducer.snapshot().calls(), 1);
        assert_eq!(counted.count().measurement().tokens(), 17);
        assert_eq!(
            counted.count().measurement().quality(),
            SemanticTokenCountQuality::ProviderExact
        );
        let structured = counted
            .input_metrics()
            .structured_input_tokens()
            .expect("provider-exact structured count");
        assert_eq!(structured.tokens(), 17);
        assert_eq!(
            structured.quality(),
            SemanticTokenCountQuality::ProviderExact
        );
        assert_eq!(
            counted
                .input_metrics()
                .semantic_payload_tokens()
                .expect("semantic metric survives exact provider count")
                .quality(),
            SemanticTokenCountQuality::Conservative
        );
        assert_eq!(
            policy.accounting().reserved_model_tokens(),
            reserved_before_count,
            "provider counting must retain the conservative policy ceiling"
        );
        let debug = format!("{counted:?}");
        assert!(!debug.contains("synthetic-openai-key"));
        assert!(!debug.contains("synthetic fixture marker"));

        let result = counted.cancel_without_model_dispatch();
        assert_eq!(result.input_metric_receipt(), counted_metric_receipt);
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::InputTokenCountDisclosed
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("count-only cancellation settles exact-zero model usage")
        };
        let receipt = settlement.settle(&mut policy).expect("policy settlement");
        assert_eq!(receipt.usage_accounting(), AgentModelUsageAccounting::Exact);
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(receipt.output_tokens(), 0);

        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head)
            .expect("request head")
            .to_ascii_lowercase();
        assert!(head.starts_with("post /v1/responses/input_tokens http/1.1\r\n"));
        assert!(head.contains("authorization: bearer synthetic-openai-key\r\n"));
        assert!(head.contains("accept: application/json\r\n"));
        assert!(head.contains("accept-encoding: identity\r\n"));
        let body: serde_json::Value = serde_json::from_slice(&captured.body).expect("count body");
        for excluded in ["max_output_tokens", "service_tier", "stream", "store"] {
            assert!(
                body.get(excluded).is_none(),
                "response-only field disclosed"
            );
        }
        for required in ["model", "instructions", "input", "tools", "tool_choice"] {
            assert!(body.get(required).is_some(), "token-relevant field omitted");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn count_overload_retry_preserves_request_and_stops_after_one_retry() {
        for recovered in [true, false] {
            let server = SequenceServer::spawn_with_statuses(vec![
                (
                    "503 Service Unavailable",
                    ScriptedResponse {
                        delay: Duration::ZERO,
                        content_type: "application/json",
                        body: b"{}".to_vec(),
                    },
                ),
                (
                    if recovered {
                        "200 OK"
                    } else {
                        "503 Service Unavailable"
                    },
                    ScriptedResponse {
                        delay: Duration::ZERO,
                        content_type: "application/json",
                        body: br#"{"object":"response.input_tokens","input_tokens":17}"#.to_vec(),
                    },
                ),
            ]);
            let transport = AgentProviderTransport::try_new_loopback(
                AgentProviderTransportConfig::STANDARD,
                server.openai.as_str(),
                server.anthropic.as_str(),
            )
            .unwrap();
            let credential = AgentProviderCredential::try_new(
                AgentProviderKind::OpenAiResponses,
                "synthetic-openai-key".into(),
            )
            .unwrap();
            let (mut policy, input) = provider_exact_fixture();
            let mut attempt = transport
                .try_admit(
                    input,
                    &mut policy,
                    &credential,
                    AgentProviderCancellation::new(),
                )
                .unwrap();
            let result = match attempt.count_openai_input_tokens().await {
                AgentProviderExactCountOutcome::Counted(counted) => {
                    assert!(recovered);
                    assert_eq!(counted.count().measurement().tokens(), 17);
                    counted.cancel_without_model_dispatch().unwrap()
                }
                AgentProviderExactCountOutcome::Failed(result) => {
                    assert!(!recovered);
                    assert!(
                        matches!(result.outcome(), AgentProviderTransportOutcome::Failed(failure) if failure.class() == AgentProviderFailureClass::Overloaded)
                    );
                    result
                }
                other => panic!("unexpected count: {other:?}"),
            };
            assert_eq!(
                result.disclosure_stage(),
                AgentProviderDisclosureStage::InputTokenCountDisclosed
            );
            assert_eq!(
                result.usage_knowledge(),
                AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
            );
            let AgentProviderPolicySettlement::Immediate(settlement) =
                result.into_policy_settlement()
            else {
                panic!("no generation");
            };
            let receipt = settlement.settle(&mut policy).unwrap();
            assert_eq!((receipt.input_tokens(), receipt.output_tokens()), (0, 0));
            assert!(transport.snapshot().unwrap().is_idle());
            let requests = server.finish();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].body, requests[1].body);
            assert_eq!(requests[0].head, requests[1].head);
            assert!(std::str::from_utf8(&requests[0].head)
                .unwrap()
                .starts_with("POST /v1/responses/input_tokens "));
        }
    }

    #[test]
    fn image_fence_revoked_before_admission_never_commits_or_reserves_a_transport_slot() {
        let transport =
            AgentProviderTransport::try_new(AgentProviderTransportConfig::STANDARD).unwrap();
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".into(),
        )
        .unwrap();
        let (mut policy, input) = provider_exact_fixture();
        let gate = lead::ModelImageGate::new(Some(true));
        let input = input.with_image_fence(gate.fence().unwrap());
        gate.begin_change().unwrap();
        assert!(matches!(
            transport.try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new()
            ),
            Err(AgentProviderAdmissionError::Cancelled)
        ));
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(policy.taints().is_empty());
        assert!(transport.snapshot().unwrap().is_idle());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn image_fence_revocation_stops_exact_count_backoff_without_retry_or_generation() {
        let server = OneShotServer::spawn(
            "503 Service Unavailable",
            &[("Retry-After", "1")],
            vec![b"{}".to_vec()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".into(),
        )
        .unwrap();
        let (mut policy, input) = provider_exact_fixture();
        let gate = lead::ModelImageGate::new(Some(true));
        // A retained visual-history fence remains mandatory even when this
        // particular fixture's current continuation body contains only text.
        let input = input.with_image_fence(gate.fence().unwrap());
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .unwrap();
        let revoke = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let epoch = gate.begin_change().unwrap();
            assert!(gate.confirm(epoch, true)); // Re-enable cannot revive it.
        });
        let AgentProviderExactCountOutcome::Failed(result) =
            attempt.count_openai_input_tokens().await
        else {
            panic!("revoked image count must stop");
        };
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("no generation");
        };
        settlement.settle(&mut policy).unwrap();
        revoke.await.unwrap();
        let request = server.finish();
        assert!(std::str::from_utf8(&request.head)
            .unwrap()
            .starts_with("POST /v1/responses/input_tokens "));
        assert!(transport.snapshot().unwrap().is_idle());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn count_overload_backoff_honors_cancellation_and_original_deadline() {
        for cancel in [true, false] {
            let server = OneShotServer::spawn(
                "503 Service Unavailable",
                &[("Retry-After", "1")],
                vec![b"{}".to_vec()],
            );
            let transport = test_transport(&server);
            let credential = AgentProviderCredential::try_new(
                AgentProviderKind::OpenAiResponses,
                "synthetic-openai-key".into(),
            )
            .unwrap();
            let (mut policy, input) = provider_exact_fixture();
            let cancellation = AgentProviderCancellation::new();
            let mut attempt = transport
                .try_admit(input, &mut policy, &credential, cancellation.clone())
                .unwrap();
            if !cancel {
                attempt.deadline = Instant::now() + Duration::from_millis(250);
            }
            let task = cancel.then(|| {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    cancellation.cancel();
                })
            });
            let AgentProviderExactCountOutcome::Failed(result) =
                attempt.count_openai_input_tokens().await
            else {
                panic!("count should stop");
            };
            assert_eq!(
                result.usage_knowledge(),
                AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
            );
            let AgentProviderPolicySettlement::Immediate(settlement) =
                result.into_policy_settlement()
            else {
                panic!("no generation");
            };
            settlement.settle(&mut policy).unwrap();
            if let Some(task) = task {
                task.await.unwrap();
            }
            let request = server.finish();
            assert!(std::str::from_utf8(&request.head)
                .unwrap()
                .starts_with("POST /v1/responses/input_tokens "));
            assert!(transport.snapshot().unwrap().is_idle());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_attempt_deadline_spans_count_and_model_requests() {
        let server = SequenceServer::spawn(vec![
            ScriptedResponse {
                delay: Duration::from_millis(600),
                content_type: "application/json",
                body: br#"{"object":"response.input_tokens","input_tokens":17}"#.to_vec(),
            },
            ScriptedResponse {
                delay: Duration::from_millis(600),
                content_type: "text/event-stream; charset=utf-8",
                body: openai_success_stream(),
            },
        ]);
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::try_new(
                // Each response fits the attempt alone; together they
                // exceed it, with slack for a slow runner.
                Duration::from_millis(1_000),
                Duration::from_millis(900),
                Duration::from_millis(900),
            )
            .expect("config"),
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_exact_fixture();
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let counted = match attempt.count_openai_input_tokens().await {
            AgentProviderExactCountOutcome::Counted(counted) => counted,
            AgentProviderExactCountOutcome::Failed(_) => panic!("count phase expected"),
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("count drive unavailable: {error:?}")
            }
        };
        let result = counted
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Timeout
        ));
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::ModelRequestMayHaveDispatched
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("ambiguous model timeout settles immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        settlement.settle(&mut policy).expect("policy settlement");

        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        let count_head = std::str::from_utf8(&requests[0].head)
            .expect("count head")
            .to_ascii_lowercase();
        let model_head = std::str::from_utf8(&requests[1].head)
            .expect("model head")
            .to_ascii_lowercase();
        assert!(count_head.starts_with("post /v1/responses/input_tokens http/1.1\r\n"));
        assert!(model_head.starts_with("post /v1/responses http/1.1\r\n"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oversized_count_response_fails_after_count_disclosure_without_model_usage() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "application/json"),
                ("Content-Encoding", "identity"),
            ],
            vec![vec![b'x'; MAX_OPENAI_INPUT_TOKEN_RESPONSE_BYTES + 1]],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_exact_fixture();
        let mut input_reducer = provider_input_metrics(&policy, 41_002);
        let result = match transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .count_openai_input_tokens()
            .await
        {
            AgentProviderExactCountOutcome::Counted(_) => panic!("oversized response accepted"),
            AgentProviderExactCountOutcome::Failed(result) => result,
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("count drive unavailable: {error:?}")
            }
        };
        let terminal_metric_receipt = result.input_metric_receipt();
        assert_eq!(
            terminal_metric_receipt
                .metrics()
                .structured_input_tokens()
                .expect("final conservative whole-request accounting")
                .quality(),
            SemanticTokenCountQuality::Conservative
        );
        input_reducer
            .record(terminal_metric_receipt)
            .expect("terminal count failure finalizes conservative input metrics");
        assert_eq!(input_reducer.snapshot().calls(), 1);
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Protocol
        ));
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::InputTokenCountDisclosed
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("count failure settles immediately")
        };
        let receipt = settlement.settle(&mut policy).expect("policy settlement");
        assert_eq!(receipt.usage_accounting(), AgentModelUsageAccounting::Exact);
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(receipt.output_tokens(), 0);
        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head)
            .expect("request head")
            .to_ascii_lowercase();
        assert!(head.starts_with("post /v1/responses/input_tokens http/1.1\r\n"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_count_above_admitted_input_reservation_fails_closed() {
        let (mut policy, input) = provider_exact_fixture();
        let over_reservation = input
            .request()
            .byte_len()
            .checked_add(1)
            .and_then(|tokens| u32::try_from(tokens).ok())
            .expect("fixture request fits bounded provider count");
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "application/json"),
                ("Content-Encoding", "identity"),
            ],
            vec![format!(
                "{{\"object\":\"response.input_tokens\",\"input_tokens\":{over_reservation}}}"
            )
            .into_bytes()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let result = match transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .count_openai_input_tokens()
            .await
        {
            AgentProviderExactCountOutcome::Counted(_) => panic!("over-budget count accepted"),
            AgentProviderExactCountOutcome::Failed(result) => result,
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("count drive unavailable: {error:?}")
            }
        };
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::InputTokenCountDisclosed
        );
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Protocol
        ));
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("over-budget count settles without model usage")
        };
        let receipt = settlement.settle(&mut policy).expect("policy settlement");
        assert_eq!(receipt.usage_accounting(), AgentModelUsageAccounting::Exact);
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(receipt.output_tokens(), 0);
        let _captured = server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn provider_count_below_catalog_pricing_range_never_dispatches_generation() {
        let fixture = stateful_provider_fixture_with_accounting(
            AgentProviderKind::OpenAiResponses,
            AgentProviderPricingProfile::try_for_input_range(
                AgentProviderPricingRevision::new(1).expect("pricing revision"),
                32,
                crate::MAX_AGENT_PROVIDER_EXACT_COUNTED_INPUT_TOKENS,
            )
            .expect("ranged pricing profile"),
            true,
        );
        let mut policy = fixture.policy;
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "application/json"),
                ("Content-Encoding", "identity"),
            ],
            vec![br#"{"object":"response.input_tokens","input_tokens":17}"#.to_vec()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let result = match transport
            .try_admit(
                fixture.input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .count_openai_input_tokens()
            .await
        {
            AgentProviderExactCountOutcome::Counted(_) => {
                panic!("below-range input count reached generation typestate")
            }
            AgentProviderExactCountOutcome::Failed(result) => result,
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("count drive unavailable: {error:?}")
            }
        };
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::InputTokenCountDisclosed
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        assert_eq!(
            result
                .input_metric_receipt()
                .metrics()
                .structured_input_tokens()
                .expect("final conservative reservation")
                .quality(),
            SemanticTokenCountQuality::Conservative
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("below-range count must settle before model dispatch")
        };
        let receipt = settlement
            .settle(&mut policy)
            .expect("exact-zero settlement");
        assert_eq!(receipt.usage_accounting(), AgentModelUsageAccounting::Exact);
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(receipt.output_tokens(), 0);

        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head)
            .expect("request head")
            .to_ascii_lowercase();
        assert!(head.starts_with("post /v1/responses/input_tokens http/1.1\r\n"));
        assert!(transport.snapshot().expect("snapshot").is_idle());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_cannot_dispatch_through_the_anthropic_entry_point() {
        let openai = Url::parse("http://127.0.0.1:9/v1/responses").expect("openai URL");
        let anthropic = Url::parse("http://127.0.0.1:9/v1/messages").expect("anthropic URL");
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            openai.as_str(),
            anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let result = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .execute_anthropic(|_| AgentProviderBatchDisposition::Continue)
            .await;
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::NotDispatched
        );
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Protocol
        ));
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("undispatched protocol failure settles immediately")
        };
        settlement.settle(&mut policy).expect("policy settlement");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_accounting_modes_cannot_bypass_their_dispatch_typestate() {
        let openai = Url::parse("http://127.0.0.1:9/v1/responses").expect("openai URL");
        let anthropic = Url::parse("http://127.0.0.1:9/v1/messages").expect("anthropic URL");
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            openai.as_str(),
            anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");

        let (mut exact_policy, exact_input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let exact_result = match transport
            .try_admit(
                exact_input,
                &mut exact_policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("exact admission")
            .count_openai_input_tokens()
            .await
        {
            AgentProviderExactCountOutcome::Counted(_) => {
                panic!("exact-local input reached provider counting")
            }
            AgentProviderExactCountOutcome::Failed(result) => result,
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("count drive unavailable: {error:?}")
            }
        };
        assert_eq!(
            exact_result.disclosure_stage(),
            AgentProviderDisclosureStage::NotDispatched
        );
        assert_eq!(
            exact_result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let AgentProviderPolicySettlement::Immediate(exact_settlement) =
            exact_result.into_policy_settlement()
        else {
            panic!("exact-local count bypass must settle immediately")
        };
        exact_settlement
            .settle(&mut exact_policy)
            .expect("exact-local settlement");

        let (mut provider_policy, provider_input) = provider_exact_fixture();
        let provider_result = transport
            .try_admit(
                provider_input,
                &mut provider_policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("provider-exact admission")
            .execute_openai_exact_local(|_| AgentProviderBatchDisposition::Continue)
            .await;
        assert_eq!(
            provider_result.disclosure_stage(),
            AgentProviderDisclosureStage::NotDispatched
        );
        assert_eq!(
            provider_result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let AgentProviderPolicySettlement::Immediate(provider_settlement) =
            provider_result.into_policy_settlement()
        else {
            panic!("provider-exact direct bypass must settle immediately")
        };
        provider_settlement
            .settle(&mut provider_policy)
            .expect("provider-exact settlement");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_request_is_single_copy_single_post_and_normalized() {
        let stream = openai_success_stream();
        let split = stream.len() / 2;
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![stream[..split].to_vec(), stream[split..].to_vec()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let attempt_debug = format!("{attempt:?}");
        assert!(!attempt_debug.contains("synthetic-openai-key"));
        assert!(!attempt_debug.contains("synthetic fixture marker"));
        let continuation = attempt.input_evidence().expect("input evidence").clone();
        let input_metrics = attempt.input_metrics().expect("input metrics");
        let metric_receipt = attempt
            .input_metric_receipt()
            .expect("exact-local input metrics are final before dispatch");
        assert_eq!(metric_receipt.call(), attempt.call().expect("call").call());
        assert_eq!(metric_receipt.metrics(), input_metrics);
        assert!(input_metrics.serialized_request_bytes() > 0);
        let AgentProviderSemanticInputStats::Observation(semantic_stats) = input_metrics.semantic()
        else {
            panic!("observation input metrics")
        };
        assert_eq!(semantic_stats.nodes(), 2);
        assert_eq!(
            input_metrics.semantic().disclosed_bytes(),
            semantic_stats.bytes()
        );
        assert_eq!(
            input_metrics
                .semantic_payload_tokens()
                .expect("semantic token count")
                .tokens(),
            10
        );
        assert_eq!(input_metrics.structured_input_tokens(), None);
        let metric_debug = format!("{metric_receipt:?}");
        assert!(metric_debug.contains("[redacted]"));
        assert!(!metric_debug.contains("synthetic fixture marker"));
        assert!(!metric_debug.contains("fixture.example.test"));
        let acknowledgement = continuation
            .observation_acknowledgement()
            .expect("observation continuation");
        assert_eq!(acknowledgement.observation().get(), 1);
        assert_eq!(acknowledgement.generation().get(), 1);
        assert!(continuation.read_receipt().is_none());
        let continuation_debug = format!("{continuation:?}");
        assert!(!continuation_debug.contains("synthetic fixture marker"));
        assert!(!continuation_debug.contains("fixture.example.test"));
        assert!(continuation_debug.contains("[redacted]"));
        assert_eq!(transport.snapshot().expect("snapshot").active_attempts(), 1);

        let mut text = String::new();
        let result = attempt
            .execute_openai_exact_local(|batch| {
                for delta in batch.into_deltas() {
                    text.push_str(delta.as_str());
                }
                AgentProviderBatchDisposition::Continue
            })
            .await;
        let AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Completed(
            completion,
        )) = result.outcome()
        else {
            panic!("normalized completion expected")
        };
        assert_eq!(text, "hello");
        assert_eq!(completion.stop(), AgentProviderStopReason::Completed);
        assert_eq!(completion.usage().total_tokens(), 20);
        let completed_usage = completion.usage();
        assert_eq!(
            continuation
                .observation_acknowledgement()
                .expect("retained continuation")
                .observation()
                .get(),
            1
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ProviderReported(completed_usage)
        );
        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("trusted pricing must be required")
        };
        assert_eq!(
            settlement.config().provider(),
            AgentProviderKind::OpenAiResponses
        );
        assert_eq!(settlement.config().model().as_str(), "gpt-5.6-terra");
        assert_eq!(settlement.usage(), completed_usage);
        assert_eq!(settlement.settlement(), AgentModelCallSettlement::Completed);
        let settlement_debug = format!("{settlement:?}");
        assert!(!settlement_debug.contains("gpt-5.6-terra"));
        assert!(!settlement_debug.contains("transport-test-v1"));
        assert_eq!(policy.pending_model_calls(), 1);
        let wrong_profile = AgentProviderPricingProfile::try_new(
            AgentProviderPricingRevision::new(2).expect("pricing revision"),
            settlement.config().pricing_profile().max_input_tokens(),
        )
        .expect("wrong profile");
        let wrong_schedule = fixture_pricing_schedule(
            settlement.config().provider(),
            settlement.config().model().clone(),
            settlement.config().reasoning_effort(),
            settlement.config().tokenizer().clone(),
            wrong_profile,
        )
        .expect("wrong schedule");
        let error = settlement
            .settle(&mut policy, &wrong_schedule)
            .expect_err("mismatched catalog must retain authority");
        assert_eq!(
            error.pricing_error(),
            Some(AgentProviderPricingError::Identity)
        );
        let settlement = error.into_unsettled().expect("retained settlement");
        assert_eq!(policy.pending_model_calls(), 1);
        let schedule = pricing_schedule(settlement.config());
        let terminal = settlement
            .settle(&mut policy, &schedule)
            .expect("catalog-priced policy settlement");
        assert!(!terminal.has_tool_turn());
        let receipt = terminal.receipt();
        assert_eq!(
            receipt.usage_accounting(),
            AgentModelUsageAccounting::PricedCeiling
        );
        assert_eq!(receipt.input_tokens(), 17);
        assert_eq!(receipt.output_tokens(), 3);
        assert_eq!(receipt.cost_micro_usd(), 80);
        let attribution = receipt
            .pricing_attribution()
            .expect("checked pricing attribution");
        assert_eq!(attribution.provider(), AgentProviderKind::OpenAiResponses);
        assert_eq!(attribution.pricing_revision().value(), 1);
        assert_eq!(attribution.schedule_guard(), schedule.accounting_guard());
        assert_eq!(attribution.cached_input_tokens(), 0);
        assert_eq!(attribution.cache_write_input_tokens(), 0);
        assert_eq!(attribution.reasoning_output_tokens(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());

        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head)
            .expect("request head")
            .to_ascii_lowercase();
        assert!(head.starts_with("post /v1/responses http/1.1\r\n"));
        assert!(head.contains("authorization: bearer synthetic-openai-key\r\n"));
        assert!(head.contains("accept: text/event-stream\r\n"));
        assert!(head.contains("accept-encoding: identity\r\n"));
        assert!(!head.contains("x-api-key"));
        let body: serde_json::Value = serde_json::from_slice(&captured.body).expect("request body");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["service_tier"], "default");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exact_single_tool_terminal_retains_one_shot_observation_seed() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![openai_tool_stream()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");

        let result = attempt
            .execute(|batch| {
                assert!(
                    batch.deltas().is_empty(),
                    "tool authority escaped before EOF"
                );
                AgentProviderBatchDisposition::Continue
            })
            .await;
        let AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Completed(
            completion,
        )) = result.outcome()
        else {
            panic!("normalized tool completion expected")
        };
        assert_eq!(completion.stop(), AgentProviderStopReason::ToolCalls);
        assert_eq!(completion.stats().tool_calls(), 1);
        let completed_call = completion.call();
        assert!(!format!("{result:?}").contains("resp_tool_1"));

        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("reported tool usage must be priced")
        };
        let wrong_profile = AgentProviderPricingProfile::try_new(
            AgentProviderPricingRevision::new(2).expect("pricing revision"),
            settlement.config().pricing_profile().max_input_tokens(),
        )
        .expect("wrong profile");
        let wrong_schedule = fixture_pricing_schedule(
            settlement.config().provider(),
            settlement.config().model().clone(),
            settlement.config().reasoning_effort(),
            settlement.config().tokenizer().clone(),
            wrong_profile,
        )
        .expect("wrong schedule");
        let refusal = settlement
            .settle(&mut policy, &wrong_schedule)
            .expect_err("unbound pricing cannot release a tool turn");
        assert_eq!(
            refusal.pricing_error(),
            Some(AgentProviderPricingError::Identity)
        );
        let settlement = refusal
            .into_unsettled()
            .expect("pricing refusal retains sealed authority");
        assert_eq!(policy.pending_model_calls(), 1);
        let schedule = pricing_schedule(settlement.config());
        let terminal = settlement
            .settle(&mut policy, &schedule)
            .expect("catalog-priced tool settlement");
        let receipt = terminal.receipt();
        assert_eq!(receipt.input_tokens(), 17);
        assert_eq!(receipt.output_tokens(), 3);
        let turn = terminal.into_tool_turn().expect("settled tool turn");
        assert_eq!(turn.proposal().kind(), AgentBrowserToolKind::Back);
        let (_, continuation) = turn.into_parts();
        assert_eq!(continuation.prior_call(), completed_call);
        assert_eq!(continuation.provider(), AgentProviderKind::OpenAiResponses);
        assert_eq!(continuation.argument_bytes(), 2);
        let continuation_debug = format!("{continuation:?}");
        assert!(!continuation_debug.contains("fc_tool_1"));
        assert!(!continuation_debug.contains("call_tool_1"));
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());

        let captured = server.finish();
        let body: serde_json::Value = serde_json::from_slice(&captured.body).expect("request body");
        assert_eq!(body["store"], false);
        assert!(body.get("previous_response_id").is_none());
    }

    async fn qualify_policy_precondition_refusal_retains_terminal_authority(
        provider: AgentProviderKind,
    ) {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![provider_tool_stream(provider)],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            provider,
            match provider {
                AgentProviderKind::OpenAiResponses => "synthetic-openai-key",
                AgentProviderKind::AnthropicMessages => "synthetic-anthropic-key",
            }
            .to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(provider);
        let result = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .execute(|_| panic!("tool-only stream emitted a public batch"))
            .await;
        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("reported usage must reach exact pricing")
        };
        let schedule = pricing_schedule(settlement.config());

        let (mut wrong_policy, _) = provider_fixture(provider);
        let refusal = settlement
            .settle(&mut wrong_policy, &schedule)
            .expect_err("foreign policy must retain terminal authority");
        assert_eq!(
            refusal.policy_precondition_error(),
            Some(AgentPolicyError::AdmissionMismatch)
        );
        assert!(!wrong_policy.is_sealed());
        assert_eq!(wrong_policy.pending_model_calls(), 1);
        let settlement = refusal.into_unsettled().expect("terminal owner retained");

        let (mut sealed_policy, _) = provider_fixture(provider);
        sealed_policy.seal_for_test();
        let refusal = settlement
            .settle(&mut sealed_policy, &schedule)
            .expect_err("sealed policy must retain terminal authority");
        assert_eq!(
            refusal.policy_precondition_error(),
            Some(AgentPolicyError::Sealed)
        );
        assert!(sealed_policy.is_sealed());
        assert_eq!(sealed_policy.pending_model_calls(), 1);
        let settlement = refusal.into_unsettled().expect("terminal owner retained");

        let terminal = settlement
            .settle(&mut policy, &schedule)
            .expect("original live policy settles exact terminal");
        assert!(terminal.has_tool_turn());
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[test]
    fn immediate_policy_precondition_refusal_retains_terminal_authority() {
        let transport = AgentProviderTransport::try_new(
            AgentProviderTransportConfig::try_new(
                Duration::from_secs(5),
                Duration::from_secs(2),
                Duration::from_secs(2),
            )
            .expect("config"),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let AgentProviderPolicySettlement::Immediate(settlement) = attempt
            .cancel_without_dispatch()
            .expect("pre-dispatch cancellation")
            .into_policy_settlement()
        else {
            panic!("pre-dispatch cancellation must settle immediately")
        };

        let (mut wrong_policy, _) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let refusal = settlement
            .settle(&mut wrong_policy)
            .expect_err("foreign policy must retain immediate terminal authority");
        assert_eq!(
            refusal.policy_precondition_error(),
            Some(AgentPolicyError::AdmissionMismatch)
        );
        assert_eq!(wrong_policy.pending_model_calls(), 1);
        let debug = format!("{refusal:?}");
        assert!(!debug.contains("synthetic-openai-key"));
        assert!(!debug.contains("synthetic fixture marker"));
        let settlement = refusal
            .into_unsettled()
            .expect("prevalidation refusal retains immediate owner");

        let (mut sealed_policy, _) = provider_fixture(AgentProviderKind::OpenAiResponses);
        sealed_policy.seal_for_test();
        let refusal = settlement
            .settle(&mut sealed_policy)
            .expect_err("sealed policy must retain immediate terminal authority");
        assert_eq!(
            refusal.policy_precondition_error(),
            Some(AgentPolicyError::Sealed)
        );
        assert_eq!(sealed_policy.pending_model_calls(), 1);
        let settlement = refusal
            .into_unsettled()
            .expect("sealed prevalidation refusal retains immediate owner");

        let receipt = settlement
            .settle(&mut policy)
            .expect("original live policy settles recovered immediate terminal");
        assert_eq!(receipt.usage_accounting(), AgentModelUsageAccounting::Exact);
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(receipt.output_tokens(), 0);
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn immediate_reservation_ceiling_precondition_refusal_retains_full_authority() {
        let server = OneShotServer::spawn(
            "429 Too Many Requests",
            &[("Content-Type", "application/json"), ("Retry-After", "2")],
            vec![b"provider-authored-error-must-not-escape".to_vec()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let reserved = policy.accounting();
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("unknown post-dispatch usage must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );

        let (mut wrong_policy, _) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let wrong_before = wrong_policy.accounting();
        let refusal = settlement
            .settle(&mut wrong_policy)
            .expect_err("foreign policy must retain reservation-ceiling authority");
        assert_eq!(
            refusal.policy_precondition_error(),
            Some(AgentPolicyError::AdmissionMismatch)
        );
        assert!(!wrong_policy.is_sealed());
        assert_eq!(wrong_policy.accounting(), wrong_before);
        let settlement = refusal
            .into_unsettled()
            .expect("foreign prevalidation refusal retains immediate owner");

        let (mut sealed_policy, _) = provider_fixture(AgentProviderKind::OpenAiResponses);
        sealed_policy.seal_for_test();
        let refusal = settlement
            .settle(&mut sealed_policy)
            .expect_err("sealed policy must retain reservation-ceiling authority");
        assert_eq!(
            refusal.policy_precondition_error(),
            Some(AgentPolicyError::Sealed)
        );
        let settlement = refusal
            .into_unsettled()
            .expect("sealed prevalidation refusal retains immediate owner");

        let receipt = settlement
            .settle(&mut policy)
            .expect("original live policy settles recovered reservation ceiling");
        assert_eq!(
            receipt.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        assert_eq!(receipt.input_tokens(), 18);
        assert_eq!(receipt.output_tokens(), 20);
        assert_eq!(receipt.cost_micro_usd(), reserved.reserved_cost_micro_usd());
        assert_eq!(
            receipt.input_tokens() + receipt.output_tokens(),
            reserved.reserved_model_tokens()
        );
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_wrong_and_sealed_policy_refusals_are_recoverable() {
        qualify_policy_precondition_refusal_retains_terminal_authority(
            AgentProviderKind::OpenAiResponses,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_wrong_and_sealed_policy_refusals_are_recoverable() {
        qualify_policy_precondition_refusal_retains_terminal_authority(
            AgentProviderKind::AnthropicMessages,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unpriceable_exact_terminal_has_one_reservation_ceiling_exit() {
        let pricing_profile = AgentProviderPricingProfile::try_for_input_range(
            AgentProviderPricingRevision::new(1).expect("pricing revision"),
            18,
            16_384,
        )
        .expect("pricing profile");
        let fixture = stateful_provider_fixture_with_profile(
            AgentProviderKind::OpenAiResponses,
            pricing_profile,
        );
        let StatefulProviderFixture {
            mut policy, input, ..
        } = fixture;
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![openai_tool_stream()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let result = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .execute(|_| panic!("tool-only stream emitted a public batch"))
            .await;
        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("reported usage must reach exact pricing")
        };
        let schedule = pricing_schedule(settlement.config());
        let refusal = settlement
            .settle(&mut policy, &schedule)
            .expect_err("authenticated usage falls outside the exact schedule range");
        assert_eq!(
            refusal.pricing_error(),
            Some(AgentProviderPricingError::InputRange)
        );
        assert_eq!(policy.pending_model_calls(), 1);
        let settlement = refusal.into_unsettled().expect("terminal owner retained");
        let receipt = settlement
            .settle_at_reservation_ceiling(&mut policy)
            .expect("unpriceable terminal has one conservative settlement exit");
        assert_eq!(
            receipt.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        assert_eq!(
            receipt.settlement(),
            AgentModelCallSettlement::ProviderFailed
        );
        assert_eq!(receipt.input_tokens(), 18);
        assert_eq!(receipt.output_tokens(), 20);
        assert_eq!(receipt.cost_micro_usd(), 100);
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    async fn qualify_tool_authority_waits_for_transport_eof(provider: AgentProviderKind) {
        let server = HeldEofServer::spawn(provider_tool_stream(provider));
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::try_new(
                Duration::from_secs(5),
                Duration::from_secs(2),
                Duration::from_secs(2),
            )
            .expect("config"),
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            provider,
            match provider {
                AgentProviderKind::OpenAiResponses => "synthetic-openai-key",
                AgentProviderKind::AnthropicMessages => "synthetic-anthropic-key",
            }
            .to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(provider);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let mut execution = tokio::spawn(async move {
            attempt
                .execute(|_| panic!("tool-only stream emitted a public batch"))
                .await
        });

        server.wait_until_terminal_sent();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut execution)
                .await
                .is_err(),
            "terminal SSE must not complete before transport-observed body EOF"
        );
        server.release_eof();
        let result = execution.await.expect("transport task");
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Completed(
                completion
            )) if completion.stop() == AgentProviderStopReason::ToolCalls
        ));
        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("EOF tool usage must be priced")
        };
        let schedule = pricing_schedule(settlement.config());
        let terminal = settlement
            .settle(&mut policy, &schedule)
            .expect("priced EOF settlement");
        assert!(terminal.into_tool_turn().is_some());
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_tool_authority_waits_for_transport_observed_eof() {
        qualify_tool_authority_waits_for_transport_eof(AgentProviderKind::OpenAiResponses).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_tool_authority_waits_for_transport_observed_eof() {
        qualify_tool_authority_waits_for_transport_eof(AgentProviderKind::AnthropicMessages).await;
    }

    async fn qualify_trailing_event_revokes_terminal_authority(provider: AgentProviderKind) {
        let mut body = provider_tool_stream(provider);
        body.extend_from_slice(b"event: ping\ndata: {\"type\":\"ping\"}\n\n");
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![body],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            provider,
            match provider {
                AgentProviderKind::OpenAiResponses => "synthetic-openai-key",
                AgentProviderKind::AnthropicMessages => "synthetic-anthropic-key",
            }
            .to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(provider);
        let result = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .execute(|_| panic!("tool-only stream emitted a public batch"))
            .await;
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Protocol
        ));
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("trailing event must discard terminal usage and authority")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        settlement
            .settle(&mut policy)
            .expect("conservative protocol settlement");
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_trailing_event_revokes_terminal_tool_authority() {
        qualify_trailing_event_revokes_terminal_authority(AgentProviderKind::OpenAiResponses).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_trailing_event_revokes_terminal_tool_authority() {
        qualify_trailing_event_revokes_terminal_authority(AgentProviderKind::AnthropicMessages)
            .await;
    }

    async fn qualify_missing_seed_cannot_release_tool(provider: AgentProviderKind) {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![provider_tool_stream(provider)],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            provider,
            match provider {
                AgentProviderKind::OpenAiResponses => "synthetic-openai-key",
                AgentProviderKind::AnthropicMessages => "synthetic-anthropic-key",
            }
            .to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(provider);
        let mut result = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .execute(|_| panic!("tool-only stream emitted a public batch"))
            .await;
        assert!(result.continuation.take().is_some(), "fixture seed");
        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("tool usage must be priced")
        };
        let schedule = pricing_schedule(settlement.config());
        let terminal = settlement
            .settle(&mut policy, &schedule)
            .expect("missing-seed failure is still exactly priced");
        assert_eq!(
            terminal.receipt().settlement(),
            AgentModelCallSettlement::ProviderFailed
        );
        assert!(!terminal.has_tool_turn());
        assert!(terminal.into_tool_turn().is_none());
        assert_eq!(policy.pending_model_calls(), 0);
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_missing_seed_cannot_release_tool_authority() {
        qualify_missing_seed_cannot_release_tool(AgentProviderKind::OpenAiResponses).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_missing_seed_cannot_release_tool_authority() {
        qualify_missing_seed_cannot_release_tool(AgentProviderKind::AnthropicMessages).await;
    }

    async fn qualify_policy_refusal_consumes_tool_authority(provider: AgentProviderKind) {
        let body = match provider {
            AgentProviderKind::OpenAiResponses => openai_tool_stream(),
            AgentProviderKind::AnthropicMessages => anthropic_tool_stream(),
        };
        let body = String::from_utf8(body).expect("fixture UTF-8");
        let body = match provider {
            AgentProviderKind::OpenAiResponses => body
                .replace("\"input_tokens\":17", "\"input_tokens\":19")
                .replace("\"total_tokens\":20", "\"total_tokens\":22"),
            AgentProviderKind::AnthropicMessages => {
                body.replace("\"input_tokens\":7", "\"input_tokens\":20")
            }
        }
        .into_bytes();
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![body],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            provider,
            match provider {
                AgentProviderKind::OpenAiResponses => "synthetic-openai-key",
                AgentProviderKind::AnthropicMessages => "synthetic-anthropic-key",
            }
            .to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(provider);
        let result = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .execute(|_| panic!("tool-only stream emitted a public batch"))
            .await;
        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("reported usage must reach exact pricing")
        };
        let schedule = pricing_schedule(settlement.config());
        let error = settlement
            .settle(&mut policy, &schedule)
            .expect_err("policy overage must consume proposal authority");
        assert!(matches!(
            error,
            AgentProviderPricingSettlementError::Policy(AgentPolicyError::ProviderUsageExceeded)
        ));
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_policy_refusal_consumes_tool_authority() {
        qualify_policy_refusal_consumes_tool_authority(AgentProviderKind::OpenAiResponses).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_policy_refusal_consumes_tool_authority() {
        qualify_policy_refusal_consumes_tool_authority(AgentProviderKind::AnthropicMessages).await;
    }

    async fn qualify_admitted_diff_transport(provider: AgentProviderKind) {
        let initial_server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![provider_tool_stream(provider)],
        );
        let initial_transport = test_transport(&initial_server);
        let credential = AgentProviderCredential::try_new(
            provider,
            match provider {
                AgentProviderKind::OpenAiResponses => "synthetic-openai-key",
                AgentProviderKind::AnthropicMessages => "synthetic-anthropic-key",
            }
            .to_owned(),
        )
        .expect("credential");
        let mut fixture = stateful_provider_fixture(provider);
        let initial_result = initial_transport
            .try_admit(
                fixture.input,
                &mut fixture.policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("initial admission")
            .execute(|batch| {
                assert!(batch.deltas().is_empty(), "tool escaped before EOF");
                AgentProviderBatchDisposition::Continue
            })
            .await;
        let AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Completed(
            initial_completion,
        )) = initial_result.outcome()
        else {
            panic!("initial tool completion expected")
        };
        assert_eq!(
            initial_completion.stop(),
            AgentProviderStopReason::ToolCalls
        );
        let initial_call = initial_completion.call();
        let AgentProviderPolicySettlement::PricingRequired(initial_settlement) =
            initial_result.into_policy_settlement()
        else {
            panic!("initial reported usage must be priced")
        };
        let initial_schedule = pricing_schedule(initial_settlement.config());
        let initial_terminal = initial_settlement
            .settle(&mut fixture.policy, &initial_schedule)
            .expect("initial catalog-priced settlement");
        let (_, continuation) = initial_terminal
            .into_tool_turn()
            .expect("settled initial tool")
            .into_parts();
        assert_eq!(continuation.prior_call(), initial_call);
        assert_eq!(fixture.policy.pending_model_calls(), 0);
        assert!(initial_transport
            .snapshot()
            .expect("initial snapshot")
            .is_idle());
        initial_server.finish();

        let current = successor_observation(&fixture.observation);
        let diff = match compute_semantic_diff(
            &fixture.observation,
            continuation.baseline(),
            &current,
            SemanticDiffBudget::ACTION,
        ) {
            SemanticDiffOutcome::Diff(diff) => diff,
            SemanticDiffOutcome::FreshSnapshot(reason) => {
                panic!("unexpected fresh snapshot: {reason:?}")
            }
        };
        let diff_payload =
            encode_semantic_diff(&diff, SemanticModelEncodingBudget::ACTION_DIFF_EXACT)
                .expect("encode diff")
                .admit(
                    &FixedCounter {
                        revision: fixture.config.tokenizer().clone(),
                        tokens: 10,
                    },
                    fixture.config.tokenizer(),
                )
                .expect("admit diff");
        let request = AgentModelCallRequest::new(
            AgentModelCallId::new(2).expect("diff call"),
            fixture.lease,
            fixture.account,
            AgentModelCallBudget::try_new(20, 20, 100).expect("diff call budget"),
            AgentPolicyInstant::from_millis(NOW),
        );
        let draft = AgentProviderDiffRequestDraft::try_new(
            continuation
                .bind_diff_request(request, &fixture.config, &diff, diff_payload)
                .expect("bind diff request"),
        )
        .expect("encode stateless diff request");
        let structured_input_tokens = match provider {
            AgentProviderKind::OpenAiResponses => 17,
            AgentProviderKind::AnthropicMessages => 15,
        };
        let prepared = draft
            .try_prepare(
                &mut fixture.policy,
                request,
                &diff,
                &FixedStructuredCounter {
                    revision: fixture.config.tokenizer().clone(),
                    tokens: structured_input_tokens,
                },
            )
            .expect("admit exact whole diff request");
        assert_eq!(
            prepared.structured_input_measurement().tokens(),
            structured_input_tokens
        );
        assert_eq!(
            fixture.policy.accounting().reserved_model_tokens(),
            u64::from(structured_input_tokens) + 20
        );

        let diff_server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![provider_tool_stream(provider)],
        );
        let diff_transport = test_transport(&diff_server);
        let mut diff_attempt = diff_transport
            .try_admit(
                prepared.into_transport_input(),
                &mut fixture.policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("diff transport admission");
        let committed_acknowledgement = diff_attempt
            .input_evidence()
            .expect("diff evidence")
            .diff_receipt()
            .expect("committed diff receipt")
            .acknowledgement()
            .clone();
        assert_eq!(
            committed_acknowledgement.observation(),
            current.request().id()
        );
        assert_eq!(
            committed_acknowledgement.generation(),
            current.request().generation()
        );
        assert_eq!(
            committed_acknowledgement.context(),
            current.request().context()
        );
        assert_eq!(
            diff_attempt
                .input_evidence()
                .expect("diff evidence")
                .observation_acknowledgement()
                .expect("diff acknowledgement"),
            &committed_acknowledgement
        );
        assert_eq!(
            fixture.policy.accounting().reserved_model_tokens(),
            u64::from(structured_input_tokens) + 20
        );

        let diff_result = diff_attempt
            .execute(|batch| {
                assert!(batch.deltas().is_empty(), "tool escaped before EOF");
                AgentProviderBatchDisposition::Continue
            })
            .await;
        let AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Completed(
            diff_completion,
        )) = diff_result.outcome()
        else {
            panic!("diff tool completion expected")
        };
        assert_eq!(diff_completion.stop(), AgentProviderStopReason::ToolCalls);
        assert!(!format!("{diff_result:?}").contains("updated synthetic marker"));
        let AgentProviderPolicySettlement::PricingRequired(diff_settlement) =
            diff_result.into_policy_settlement()
        else {
            panic!("diff reported usage must be priced")
        };
        let diff_schedule = pricing_schedule(diff_settlement.config());
        let diff_terminal = diff_settlement
            .settle(&mut fixture.policy, &diff_schedule)
            .expect("diff catalog-priced settlement");
        let receipt = diff_terminal.receipt();
        let (_, next_continuation) = diff_terminal
            .into_tool_turn()
            .expect("settled diff tool")
            .into_parts();
        assert_eq!(next_continuation.baseline(), &committed_acknowledgement);
        assert_eq!(receipt.input_tokens(), u64::from(structured_input_tokens));
        assert_eq!(
            receipt.output_tokens(),
            match provider {
                AgentProviderKind::OpenAiResponses => 3,
                AgentProviderKind::AnthropicMessages => 4,
            }
        );
        assert_eq!(fixture.policy.pending_model_calls(), 0);
        assert!(diff_transport.snapshot().expect("diff snapshot").is_idle());

        let captured = diff_server.finish();
        let body: serde_json::Value =
            serde_json::from_slice(&captured.body).expect("diff request body");
        match provider {
            AgentProviderKind::OpenAiResponses => {
                assert_eq!(body["store"], false);
                assert!(body.get("previous_response_id").is_none());
                let input = body["input"].as_array().expect("fixed OpenAI input");
                assert_eq!(input.len(), 4);
                assert_eq!(input[2]["type"], "function_call");
                assert_eq!(input[3]["type"], "function_call_output");
                assert_eq!(input[2]["call_id"], input[3]["call_id"]);
            }
            AgentProviderKind::AnthropicMessages => {
                let messages = body["messages"]
                    .as_array()
                    .expect("fixed Anthropic messages");
                assert_eq!(messages.len(), 3);
                assert_eq!(messages[1]["role"], "assistant");
                assert_eq!(messages[1]["content"][0]["type"], "tool_use");
                assert_eq!(messages[2]["role"], "user");
                assert_eq!(messages[2]["content"][0]["type"], "tool_result");
                assert_eq!(
                    messages[1]["content"][0]["id"],
                    messages[2]["content"][0]["tool_use_id"]
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_diff_is_single_post_and_retains_the_next_exact_seed() {
        qualify_admitted_diff_transport(AgentProviderKind::OpenAiResponses).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_provider_exact_initial_and_diff_publish_only_post_count_metrics() {
        let mut fixture = stateful_provider_fixture_with_accounting(
            AgentProviderKind::OpenAiResponses,
            AgentProviderPricingProfile::try_new(
                AgentProviderPricingRevision::new(1).expect("pricing revision"),
                crate::MAX_AGENT_PROVIDER_EXACT_COUNTED_INPUT_TOKENS,
            )
            .expect("pricing profile"),
            true,
        );
        let mut input_reducer = provider_input_metrics(&fixture.policy, 51_001);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let initial_server = SequenceServer::spawn(vec![
            ScriptedResponse {
                delay: Duration::ZERO,
                content_type: "application/json",
                body: br#"{"object":"response.input_tokens","input_tokens":17}"#.to_vec(),
            },
            ScriptedResponse {
                delay: Duration::ZERO,
                content_type: "text/event-stream; charset=utf-8",
                body: openai_tool_stream(),
            },
        ]);
        let initial_transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            initial_server.openai.as_str(),
            initial_server.anthropic.as_str(),
        )
        .expect("initial transport");
        let mut initial_attempt = initial_transport
            .try_admit(
                fixture.input,
                &mut fixture.policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("initial admission");
        assert!(initial_attempt.input_metric_receipt().is_none());
        let initial_counted = match initial_attempt.count_openai_input_tokens().await {
            AgentProviderExactCountOutcome::Counted(counted) => counted,
            AgentProviderExactCountOutcome::Failed(result) => {
                panic!("initial count failed: {result:?}")
            }
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("initial count unavailable: {error:?}")
            }
        };
        let initial_metric_receipt = initial_counted.input_metric_receipt();
        input_reducer
            .record(initial_metric_receipt)
            .expect("record initial post-count metrics");
        let initial_result = initial_counted
            .execute(|batch| {
                assert!(batch.deltas().is_empty());
                AgentProviderBatchDisposition::Continue
            })
            .await;
        assert_eq!(
            initial_result.input_metric_receipt(),
            initial_metric_receipt
        );
        let AgentProviderPolicySettlement::PricingRequired(initial_settlement) =
            initial_result.into_policy_settlement()
        else {
            panic!("initial usage must be priced")
        };
        let initial_schedule = pricing_schedule(initial_settlement.config());
        let initial_terminal = initial_settlement
            .settle(&mut fixture.policy, &initial_schedule)
            .expect("initial settlement");
        let (_, continuation) = initial_terminal
            .into_tool_turn()
            .expect("initial tool turn")
            .into_parts();
        let initial_requests = initial_server.finish();
        assert_eq!(initial_requests.len(), 2);

        let current = successor_observation(&fixture.observation);
        let diff = match compute_semantic_diff(
            &fixture.observation,
            continuation.baseline(),
            &current,
            SemanticDiffBudget::ACTION,
        ) {
            SemanticDiffOutcome::Diff(diff) => diff,
            SemanticDiffOutcome::FreshSnapshot(reason) => {
                panic!("unexpected fresh snapshot: {reason:?}")
            }
        };
        let diff_payload = encode_semantic_diff(
            &diff,
            SemanticModelEncodingBudget::try_new(
                16 * 1024,
                1_000,
                SemanticTokenCountRequirement::ConservativeAllowed,
            )
            .expect("diff budget"),
        )
        .expect("encode diff")
        .admit_conservative_utf8(fixture.config.tokenizer())
        .expect("conservative diff");
        let request = AgentModelCallRequest::new(
            AgentModelCallId::new(2).expect("diff call"),
            fixture.lease,
            fixture.account,
            AgentModelCallBudget::try_new(
                u32::try_from(crate::MAX_AGENT_PROVIDER_EXACT_COUNTED_INPUT_TOKENS)
                    .expect("provider-exact ceiling fits u32"),
                20,
                100,
            )
            .expect("diff call budget"),
            AgentPolicyInstant::from_millis(NOW),
        );
        let draft = AgentProviderDiffRequestDraft::try_new(
            continuation
                .bind_diff_request(request, &fixture.config, &diff, diff_payload)
                .expect("bind provider-exact diff"),
        )
        .expect("encode provider-exact diff");
        let prepared = draft
            .try_prepare_for_provider_exact_count(&mut fixture.policy, request, &diff)
            .expect("reserve provider-exact diff");
        let diff_server = SequenceServer::spawn(vec![
            ScriptedResponse {
                delay: Duration::ZERO,
                content_type: "application/json",
                body: br#"{"object":"response.input_tokens","input_tokens":17}"#.to_vec(),
            },
            ScriptedResponse {
                delay: Duration::ZERO,
                content_type: "text/event-stream; charset=utf-8",
                body: openai_tool_stream(),
            },
        ]);
        let diff_transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            diff_server.openai.as_str(),
            diff_server.anthropic.as_str(),
        )
        .expect("diff transport");
        let mut diff_attempt = diff_transport
            .try_admit(
                prepared.into_transport_input(),
                &mut fixture.policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("diff admission");
        assert!(diff_attempt.input_metric_receipt().is_none());
        let diff_counted = match diff_attempt.count_openai_input_tokens().await {
            AgentProviderExactCountOutcome::Counted(counted) => counted,
            AgentProviderExactCountOutcome::Failed(result) => {
                panic!("diff count failed: {result:?}")
            }
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("diff count unavailable: {error:?}")
            }
        };
        let diff_metric_receipt = diff_counted.input_metric_receipt();
        input_reducer
            .record(diff_metric_receipt)
            .expect("record diff post-count metrics");
        let diff_result = diff_counted
            .execute(|batch| {
                assert!(batch.deltas().is_empty());
                AgentProviderBatchDisposition::Continue
            })
            .await;
        assert_eq!(diff_result.input_metric_receipt(), diff_metric_receipt);
        let AgentProviderPolicySettlement::PricingRequired(diff_settlement) =
            diff_result.into_policy_settlement()
        else {
            panic!("diff usage must be priced")
        };
        let diff_schedule = pricing_schedule(diff_settlement.config());
        let _diff_terminal = diff_settlement
            .settle(&mut fixture.policy, &diff_schedule)
            .expect("diff settlement");
        let diff_requests = diff_server.finish();
        assert_eq!(diff_requests.len(), 2);

        let snapshot = input_reducer.snapshot();
        assert_eq!(snapshot.calls(), 2);
        for kind in [
            AgentProviderInputKind::Observation,
            AgentProviderInputKind::Diff,
        ] {
            let metrics = snapshot.kind(kind);
            assert_eq!(metrics.calls(), 1);
            assert_eq!(
                metrics.structured_input_quality(SemanticTokenCountQuality::ProviderExact),
                1
            );
            assert_eq!(
                metrics.structured_input_quality(SemanticTokenCountQuality::Conservative),
                0
            );
        }
        assert_eq!(fixture.policy.pending_model_calls(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_diff_is_single_post_and_retains_the_next_exact_seed() {
        qualify_admitted_diff_transport(AgentProviderKind::AnthropicMessages).await;
    }

    async fn qualify_admitted_extraction_transport(provider: AgentProviderKind) {
        let initial_server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![provider_extract_tool_stream(provider)],
        );
        let initial_transport = test_transport(&initial_server);
        let credential = AgentProviderCredential::try_new(
            provider,
            match provider {
                AgentProviderKind::OpenAiResponses => "synthetic-openai-key",
                AgentProviderKind::AnthropicMessages => "synthetic-anthropic-key",
            }
            .to_owned(),
        )
        .expect("credential");
        let mut fixture = stateful_provider_fixture(provider);
        let initial_result = initial_transport
            .try_admit(
                fixture.input,
                &mut fixture.policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("initial extraction admission")
            .execute(|batch| {
                assert!(batch.deltas().is_empty(), "tool escaped before EOF");
                AgentProviderBatchDisposition::Continue
            })
            .await;
        let AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Completed(
            initial_completion,
        )) = initial_result.outcome()
        else {
            panic!("initial extraction tool completion expected: {initial_result:?}")
        };
        assert_eq!(
            initial_completion.stop(),
            AgentProviderStopReason::ToolCalls
        );
        let initial_call = initial_completion.call();
        let AgentProviderPolicySettlement::PricingRequired(initial_settlement) =
            initial_result.into_policy_settlement()
        else {
            panic!("initial extraction usage must be priced")
        };
        let initial_schedule = pricing_schedule(initial_settlement.config());
        let initial_terminal = initial_settlement
            .settle(&mut fixture.policy, &initial_schedule)
            .expect("initial extraction settlement");
        let (_, continuation) = initial_terminal
            .into_tool_turn()
            .expect("settled extraction tool")
            .into_parts();
        assert_eq!(continuation.prior_call(), initial_call);
        initial_server.finish();

        let read = read_semantic_observation(
            &fixture.observation,
            SemanticReadAuthority::Initial,
            SemanticCaptureInstant::from_millis(NOW - 2),
            SemanticReadSensitivityLimit::PublicOnly,
            SemanticReadBudget::STANDARD,
        )
        .expect("extraction read");
        let schema = SemanticExtractionSchema::try_new(
            SemanticExtractionSchemaId::new(71).expect("schema id"),
            vec![
                SemanticExtractionFieldSchema::try_text("title".to_owned(), true, 64)
                    .expect("title field"),
            ],
        )
        .expect("extraction schema");
        let payload = encode_semantic_extraction_request(
            &schema,
            &read,
            SemanticModelEncodingBudget::try_new(
                32 * 1024,
                32 * 1024,
                SemanticTokenCountRequirement::Exact,
            )
            .expect("extraction budget"),
        )
        .expect("encode extraction input")
        .admit(
            &FixedCounter {
                revision: fixture.config.tokenizer().clone(),
                tokens: 10,
            },
            fixture.config.tokenizer(),
        )
        .expect("admit extraction input");
        let request = AgentModelCallRequest::new(
            AgentModelCallId::new(2).expect("extraction call"),
            fixture.lease,
            fixture.account,
            AgentModelCallBudget::try_new(30, 20, 100).expect("extraction call budget"),
            AgentPolicyInstant::from_millis(NOW),
        );
        let draft = AgentProviderExtractionRequestDraft::try_new(
            continuation
                .bind_extraction_request(request, &fixture.config, &schema, &read, payload)
                .expect("bind extraction request"),
        )
        .expect("encode extraction request");
        let structured_input_tokens = match provider {
            AgentProviderKind::OpenAiResponses => 17,
            AgentProviderKind::AnthropicMessages => 15,
        };
        let prepared = draft
            .try_prepare(
                &mut fixture.policy,
                request,
                &schema,
                &read,
                &FixedStructuredCounter {
                    revision: fixture.config.tokenizer().clone(),
                    tokens: structured_input_tokens,
                },
            )
            .expect("admit exact extraction request");
        let (input, output_binding) = prepared.into_transport_parts();
        let output = r#"{"v":1,"schema":71,"fields":[{"name":"title","value":{"k":"text","value":"synthetic fixture marker","sources":["@r1"]}}]}"#;
        let output_server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![provider_extraction_output_stream(
                provider,
                output,
                structured_input_tokens,
            )],
        );
        let output_transport = test_transport(&output_server);
        let mut attempt = output_transport
            .try_admit(
                input,
                &mut fixture.policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("extraction transport admission");
        assert!(attempt
            .input_evidence()
            .expect("extraction evidence")
            .extraction_receipt()
            .is_some_and(|receipt| receipt.matches(&schema, &read)));
        let mut collector = output_binding
            .start(attempt.input_evidence().expect("extraction evidence"))
            .expect("bind committed extraction input");
        let output_result = attempt
            .execute(|batch| {
                collector
                    .push_batch(batch)
                    .expect("collect extraction stream batch");
                AgentProviderBatchDisposition::Continue
            })
            .await;
        let AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Completed(
            output_completion,
        )) = output_result.outcome()
        else {
            panic!("completed extraction output expected")
        };
        assert_eq!(output_completion.stop(), AgentProviderStopReason::Completed);
        assert_eq!(collector.retained_bytes(), output.len());
        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            output_result.into_policy_settlement()
        else {
            panic!("extraction usage must be priced")
        };
        let schedule = pricing_schedule(settlement.config());
        let terminal = settlement
            .settle(&mut fixture.policy, &schedule)
            .expect("extraction pricing settlement");
        assert!(!terminal.has_tool_turn());
        let receipt = terminal.receipt();
        assert_eq!(receipt.input_tokens(), u64::from(structured_input_tokens));
        let extracted = collector
            .finish(
                &terminal,
                &schema,
                &read,
                SemanticReadSensitivityLimit::PublicOnly,
            )
            .expect("admit extracted output");
        assert_eq!(extracted.schema(), schema.id());
        assert_eq!(extracted.stats().fields(), 1);
        assert_eq!(extracted.stats().source_edges(), 1);
        assert_eq!(fixture.policy.taints().len(), 1);
        assert_eq!(fixture.policy.pending_model_calls(), 0);
        assert!(output_transport
            .snapshot()
            .expect("extraction snapshot")
            .is_idle());

        let captured = output_server.finish();
        let body: serde_json::Value =
            serde_json::from_slice(&captured.body).expect("extraction request body");
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
        let format_schema = match provider {
            AgentProviderKind::OpenAiResponses => {
                assert_eq!(body["text"]["format"]["type"], "json_schema");
                assert_eq!(body["text"]["format"]["strict"], true);
                assert_eq!(body["input"].as_array().unwrap().len(), 2);
                assert_eq!(body["input"][1]["role"], "user");
                assert!(body["input"][1]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("ZEXTRACT1"));
                &body["text"]["format"]["schema"]
            }
            AgentProviderKind::AnthropicMessages => {
                assert_eq!(body["output_config"]["format"]["type"], "json_schema");
                assert_eq!(body["messages"].as_array().unwrap().len(), 1);
                assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 2);
                assert!(body["messages"][0]["content"][1]["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("ZEXTRACT1"));
                &body["output_config"]["format"]["schema"]
            }
        };
        // Trusted field names are intentionally bound into the schema; they
        // are values, not schema metadata or an unbounded output contract.
        assert_eq!(
            format_schema["properties"]["schema"]["enum"],
            serde_json::json!([71])
        );
        let fields = &format_schema["properties"]["fields"];
        let variants = fields["items"]["anyOf"].as_array().expect("bound fields");
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0]["additionalProperties"], false);
        assert_eq!(
            variants[0]["properties"]["name"]["enum"],
            serde_json::json!(["title"])
        );
        assert_eq!(
            variants[0]["properties"]["value"]["properties"]["k"]["enum"],
            serde_json::json!(["text"])
        );
        let maximum = &variants[0]["properties"]["value"]["properties"]["value"]["maxLength"];
        match provider {
            AgentProviderKind::OpenAiResponses => {
                assert_eq!(fields["minItems"], 1);
                assert_eq!(fields["maxItems"], 1);
                assert_eq!(maximum, 64);
            }
            AgentProviderKind::AnthropicMessages => {
                // The existing Anthropic projection omits unsupported numeric
                // schema bounds; the same Rust result admission enforces them.
                assert!(fields.get("minItems").is_none());
                assert!(fields.get("maxItems").is_none());
                assert!(maximum.is_null());
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_extraction_is_constrained_single_post_and_terminal() {
        qualify_admitted_extraction_transport(AgentProviderKind::OpenAiResponses).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_extraction_is_constrained_single_post_and_terminal() {
        qualify_admitted_extraction_transport(AgentProviderKind::AnthropicMessages).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mixed_assistant_text_and_tool_terminal_fails_closed_without_continuation() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![openai_mixed_tool_stream()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let result = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission")
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Protocol
        ));
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::ModelRequestMayHaveDispatched
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("untrusted mixed output must settle conservatively")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        settlement
            .settle(&mut policy)
            .expect("conservative mixed-output settlement");
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn terminal_failure_with_usage_retains_catalog_ceiling_accounting() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream"),
                ("Content-Encoding", "identity"),
            ],
            vec![anthropic_failure_stream()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::AnthropicMessages,
            "synthetic-anthropic-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::AnthropicMessages);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        let AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Failed(failure)) =
            result.outcome()
        else {
            panic!("normalized terminal failure expected")
        };
        assert_eq!(
            failure.failure().class(),
            AgentProviderFailureClass::Overloaded
        );
        let usage = failure.usage().expect("provider usage");
        assert_eq!(usage.input_tokens(), 15);
        assert_eq!(usage.output_tokens(), 1);
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ProviderReported(usage)
        );
        assert!(!format!("{result:?}").contains("provider-authored-sensitive-detail"));

        let AgentProviderPolicySettlement::PricingRequired(settlement) =
            result.into_policy_settlement()
        else {
            panic!("reported failure usage must be priced")
        };
        assert_eq!(
            settlement.config().provider(),
            AgentProviderKind::AnthropicMessages
        );
        assert_eq!(
            settlement.settlement(),
            AgentModelCallSettlement::ProviderFailed
        );
        let schedule = pricing_schedule(settlement.config());
        let terminal = settlement
            .settle(&mut policy, &schedule)
            .expect("priced failure settlement");
        let receipt = terminal.receipt();
        assert_eq!(
            receipt.usage_accounting(),
            AgentModelUsageAccounting::PricedCeiling
        );
        assert_eq!(receipt.input_tokens(), 15);
        assert_eq!(receipt.output_tokens(), 1);
        assert_eq!(receipt.cost_micro_usd(), 25);
        let attribution = receipt
            .pricing_attribution()
            .expect("checked pricing attribution");
        assert_eq!(attribution.provider(), AgentProviderKind::AnthropicMessages);
        assert_eq!(attribution.schedule_guard(), schedule.accounting_guard());
        assert_eq!(attribution.cached_input_tokens(), 5);
        assert_eq!(attribution.cache_write_input_tokens(), 3);
        assert_eq!(attribution.reasoning_output_tokens(), 0);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn terminal_failure_without_usage_consumes_the_reservation_ceiling() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[("Content-Type", "text/event-stream")],
            vec![anthropic_failure_without_usage_stream()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::AnthropicMessages,
            "synthetic-anthropic-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::AnthropicMessages);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Stream(AgentProviderStreamConclusion::Failed(failure))
                if failure.usage().is_none()
                    && failure.failure().class() == AgentProviderFailureClass::Overloaded
        ));
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("missing terminal usage must settle conservatively")
        };
        let receipt = settlement
            .settle(&mut policy)
            .expect("reservation-ceiling settlement");
        assert_eq!(
            receipt.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        assert_eq!(receipt.input_tokens(), 18);
        assert_eq!(receipt.output_tokens(), 20);
        assert_eq!(receipt.cost_micro_usd(), 100);
        assert!(transport.snapshot().expect("snapshot").is_idle());
        server.finish();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn anthropic_status_is_closed_retry_hint_and_never_reads_error_body() {
        let server = OneShotServer::spawn(
            "429 Too Many Requests",
            &[("Content-Type", "application/json"), ("Retry-After", "2")],
            vec![b"provider-authored-error-must-not-escape".to_vec()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::AnthropicMessages,
            "synthetic-anthropic-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::AnthropicMessages);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        let AgentProviderTransportOutcome::Failed(failure) = result.outcome() else {
            panic!("HTTP failure expected")
        };
        assert_eq!(failure.class(), AgentProviderFailureClass::RateLimited);
        assert_eq!(
            failure.retry_after().map(AgentProviderRetryAfter::millis),
            Some(2_000)
        );
        let rendered = format!("{result:?}");
        assert!(!rendered.contains("provider-authored-error"));
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("unknown usage must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        assert_eq!(
            settlement.settlement(),
            AgentModelCallSettlement::ProviderFailed
        );
        let receipt = settlement
            .settle(&mut policy)
            .expect("conservative settlement");
        assert_eq!(receipt.input_tokens(), 18);
        assert_eq!(receipt.output_tokens(), 20);
        assert_eq!(receipt.cost_micro_usd(), 100);

        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head)
            .expect("request head")
            .to_ascii_lowercase();
        assert!(head.starts_with("post /v1/messages http/1.1\r\n"));
        assert!(head.contains("x-api-key: synthetic-anthropic-key\r\n"));
        assert!(head.contains("anthropic-version: 2023-06-01\r\n"));
        assert!(!head.contains("authorization:"));
        let body: serde_json::Value = serde_json::from_slice(&captured.body).expect("request body");
        assert_eq!(body["service_tier"], "standard_only");
        assert_eq!(body["inference_geo"], "global");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn low_level_connection_failure_is_never_automatically_retried() {
        let server = DropConnectionServer::spawn();
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::try_new(
                Duration::from_secs(3),
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .expect("config"),
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        let AgentProviderTransportOutcome::Failed(failure) = result.outcome() else {
            panic!("transport failure expected")
        };
        assert_eq!(failure.class(), AgentProviderFailureClass::Transport);
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("unknown usage must settle immediately")
        };
        let receipt = settlement
            .settle(&mut policy)
            .expect("conservative settlement");
        assert_eq!(
            receipt.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        assert_eq!(server.finish(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn redirect_response_is_not_followed() {
        let server = OneShotServer::spawn(
            "307 Temporary Redirect",
            &[("Location", "http://127.0.0.1:9/must-not-follow")],
            Vec::new(),
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        let AgentProviderTransportOutcome::Failed(failure) = result.outcome() else {
            panic!("protocol failure expected")
        };
        assert_eq!(failure.class(), AgentProviderFailureClass::Protocol);
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("unknown usage must settle immediately")
        };
        settlement
            .settle(&mut policy)
            .expect("conservative settlement");
        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head).expect("request head");
        assert!(head.starts_with("POST /v1/responses HTTP/1.1\r\n"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellation_capacity_duplicate_and_shutdown_are_exact() {
        let openai = Url::parse("http://127.0.0.1:9/v1/responses").expect("openai URL");
        let anthropic = Url::parse("http://127.0.0.1:9/v1/messages").expect("anthropic URL");
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            openai.as_str(),
            anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");

        let (mut cancelled_policy, cancelled_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        let cancelled = AgentProviderCancellation::new();
        cancelled.cancel();
        assert!(matches!(
            transport.try_admit(
                cancelled_input,
                &mut cancelled_policy,
                &credential,
                cancelled
            ),
            Err(AgentProviderAdmissionError::Cancelled)
        ));
        assert_eq!(cancelled_policy.pending_model_calls(), 0);

        let mut admitted = Vec::new();
        for _ in 0..MAX_AGENT_PROVIDER_TRANSPORT_CALLS {
            let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
            let attempt = transport
                .try_admit(
                    input,
                    &mut policy,
                    &credential,
                    AgentProviderCancellation::new(),
                )
                .expect("bounded admission");
            admitted.push((attempt, policy));
        }
        assert_eq!(transport.snapshot().expect("snapshot").active_attempts(), 4);

        assert!(matches!(
            transport.reserve(admitted[0].0.call().expect("live call")),
            Err(ReserveError::Duplicate)
        ));

        let (mut excess_policy, excess_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        assert!(matches!(
            transport.try_admit(
                excess_input,
                &mut excess_policy,
                &credential,
                AgentProviderCancellation::new()
            ),
            Err(AgentProviderAdmissionError::Capacity)
        ));
        assert_eq!(excess_policy.pending_model_calls(), 0);

        for (mut attempt, mut policy) in admitted {
            let result = attempt.cancel_without_dispatch();
            assert_eq!(
                result.usage_knowledge(),
                AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
            );
            assert!(matches!(
                result.outcome(),
                AgentProviderTransportOutcome::Failed(failure)
                    if failure.class() == AgentProviderFailureClass::Cancelled
            ));
            let AgentProviderPolicySettlement::Immediate(settlement) =
                result.into_policy_settlement()
            else {
                panic!("pre-dispatch cancellation must settle immediately")
            };
            assert_eq!(
                settlement.usage_accounting(),
                AgentModelUsageAccounting::Exact
            );
            assert_eq!(settlement.settlement(), AgentModelCallSettlement::Cancelled);
            let receipt = settlement.settle(&mut policy).expect("cancel settlement");
            assert_eq!(receipt.input_tokens(), 0);
            assert_eq!(receipt.output_tokens(), 0);
            assert_eq!(receipt.cost_micro_usd(), 0);
            assert_eq!(policy.pending_model_calls(), 0);
            assert_eq!(policy.taints().len(), 1);
            let accounting = policy.accounting();
            assert_eq!(accounting.consumed_operations(), 1);
            assert_eq!(accounting.reserved_operations(), 0);
            assert_eq!(accounting.consumed_model_tokens(), 0);
            assert_eq!(accounting.consumed_cost_micro_usd(), 0);
        }
        assert!(transport.snapshot().expect("snapshot").is_idle());

        transport.seal();
        let (mut sealed_policy, sealed_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        assert!(matches!(
            transport.try_admit(
                sealed_input,
                &mut sealed_policy,
                &credential,
                AgentProviderCancellation::new()
            ),
            Err(AgentProviderAdmissionError::Sealed)
        ));
        assert_eq!(sealed_policy.pending_model_calls(), 0);
        assert!(transport.snapshot().expect("snapshot").is_sealed());
    }

    #[test]
    fn cancellation_linearizes_after_an_inflight_disclosure_gate() {
        let cancellation = AgentProviderCancellation::new();
        let commit_gate = cancellation.lock_commit_gate().expect("commit gate");
        let (attempted_sender, attempted) = mpsc::sync_channel(0);
        let (completed_sender, completed) = mpsc::sync_channel(0);
        let cancelling = cancellation.clone();
        let cancellation_thread = thread::spawn(move || {
            attempted_sender.send(()).expect("announce cancellation");
            let first = cancelling.cancel();
            completed_sender
                .send(first)
                .expect("report cancellation result");
        });

        attempted.recv().expect("cancellation attempted");
        assert!(!cancellation.is_cancelled());
        assert!(matches!(
            completed.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        drop(commit_gate);
        assert!(completed
            .recv_timeout(Duration::from_secs(1))
            .expect("cancellation completed"));
        cancellation_thread.join().expect("cancellation thread");
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn poisoned_disclosure_gate_becomes_sticky_cancellation() {
        let cancellation = AgentProviderCancellation::new();
        let poisoning = cancellation.clone();
        let _ = std::panic::catch_unwind(move || {
            let _gate = poisoning.lock_commit_gate().expect("commit gate");
            panic!("poison provider disclosure gate");
        });

        assert!(cancellation.lock_commit_gate().is_none());
        assert!(cancellation.is_cancelled());
        assert!(!cancellation.cancel());
    }

    #[test]
    fn shared_run_and_shutdown_authority_cannot_self_deadlock() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                transport.shared.shutdown.clone(),
            )
            .expect("shared-authority admission");

        let settlement = attempt.cancel_without_dispatch().into_policy_settlement();
        let AgentProviderPolicySettlement::Immediate(settlement) = settlement else {
            panic!("unpolled attempt must settle immediately")
        };
        settlement.settle(&mut policy).expect("settlement");
        assert!(transport.snapshot().expect("snapshot").is_idle());
    }

    #[test]
    fn shutdown_proof_distinguishes_open_idle_from_sealed_drain() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let shared_clone = transport.clone();
        let idle = transport.snapshot().expect("idle snapshot");
        assert!(idle.is_idle());
        assert!(!idle.is_quiescent());
        assert_eq!(
            transport
                .try_prove_shutdown()
                .expect_err("open admission cannot prove shutdown"),
            AgentProviderTransportShutdownError::Unsealed
        );

        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("attempt");
        transport.seal();
        let pending = transport.snapshot().expect("pending snapshot");
        assert!(pending.is_sealed());
        assert!(!pending.is_idle());
        assert!(!pending.is_quiescent());
        assert_eq!(
            transport
                .try_prove_shutdown()
                .expect_err("active attempt cannot prove shutdown"),
            AgentProviderTransportShutdownError::Pending
        );

        let result = attempt.cancel_without_dispatch();
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("pre-dispatch shutdown cancellation must settle immediately")
        };
        settlement
            .settle(&mut policy)
            .expect("cancelled policy settlement");
        let proof = transport
            .try_prove_shutdown()
            .expect("sealed idle transport proof");
        assert!(proof.snapshot().is_quiescent());
        assert_eq!(proof.snapshot().active_attempts(), 0);
        assert_eq!(
            format!("{proof:?}"),
            "AgentProviderTransportShutdownProof { sealed: true, active_attempts: 0 }"
        );
        assert!(
            std::mem::size_of::<AgentProviderTransportShutdownProof>()
                <= MAX_AGENT_PROVIDER_TRANSPORT_SHUTDOWN_PROOF_BYTES
        );

        let (mut post_proof_policy, post_proof_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        assert!(matches!(
            shared_clone.try_admit(
                post_proof_input,
                &mut post_proof_policy,
                &credential,
                AgentProviderCancellation::new()
            ),
            Err(AgentProviderAdmissionError::Sealed)
        ));
        assert_eq!(post_proof_policy.pending_model_calls(), 0);
        assert!(proof.snapshot().is_quiescent());
    }

    #[test]
    fn shutdown_wait_seals_before_its_future_is_polled_or_retained() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("attempt");

        let unpolled = transport
            .seal_and_prove_shutdown_until(
                Instant::now()
                    .checked_add(Duration::from_secs(1))
                    .expect("deadline"),
            )
            .expect("shutdown wait admission");
        assert!(transport.snapshot().expect("sealed snapshot").is_sealed());
        assert!(matches!(
            transport.seal_and_prove_shutdown_until(
                Instant::now()
                    .checked_add(Duration::from_secs(1))
                    .expect("deadline")
            ),
            Err(AgentProviderTransportShutdownError::WaiterActive)
        ));
        drop(unpolled);
        let readmitted_wait = transport
            .seal_and_prove_shutdown_until(
                Instant::now()
                    .checked_add(Duration::from_secs(1))
                    .expect("deadline"),
            )
            .expect("dropped wait releases bounded admission");
        drop(readmitted_wait);
        assert!(matches!(
            transport.try_prove_shutdown(),
            Err(AgentProviderTransportShutdownError::Pending)
        ));

        let result = attempt.cancel_without_dispatch();
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("pre-dispatch shutdown cancellation must settle immediately")
        };
        settlement
            .settle(&mut policy)
            .expect("cancelled policy settlement");
        assert!(transport
            .try_prove_shutdown()
            .expect("cancelled future leaves sticky seal")
            .snapshot()
            .is_quiescent());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_wait_wakes_on_last_slot_and_deadline_refuses_pending_drain() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut first_policy, first_input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let (mut second_policy, second_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        let first = transport
            .try_admit(
                first_input,
                &mut first_policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("first attempt");
        let second = transport
            .try_admit(
                second_input,
                &mut second_policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("second attempt");
        let shutdown = transport
            .seal_and_prove_shutdown_until(
                Instant::now()
                    .checked_add(Duration::from_secs(1))
                    .expect("deadline"),
            )
            .expect("shutdown wait admission");

        let settle_attempts = async move {
            tokio::task::yield_now().await;
            for (mut attempt, policy) in [(first, &mut first_policy), (second, &mut second_policy)]
            {
                let result = attempt.cancel_without_dispatch();
                let AgentProviderPolicySettlement::Immediate(settlement) =
                    result.into_policy_settlement()
                else {
                    panic!("pre-dispatch shutdown cancellation must settle immediately")
                };
                settlement
                    .settle(policy)
                    .expect("cancelled policy settlement");
            }
        };
        let (proof, ()) = tokio::join!(shutdown, settle_attempts);
        assert!(proof
            .expect("last slot wakes shutdown waiter")
            .snapshot()
            .is_quiescent());

        let pending = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("pending transport");
        let (mut pending_policy, pending_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut pending_attempt = pending
            .try_admit(
                pending_input,
                &mut pending_policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("pending attempt");
        assert_eq!(
            pending
                .seal_and_prove_shutdown_until(Instant::now())
                .expect("shutdown wait admission")
                .await
                .expect_err("elapsed deadline cannot prove pending drain"),
            AgentProviderTransportShutdownError::Deadline
        );
        assert!(pending.snapshot().expect("pending snapshot").is_sealed());
        let result = pending_attempt.cancel_without_dispatch();
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("pre-dispatch shutdown cancellation must settle immediately")
        };
        settlement
            .settle(&mut pending_policy)
            .expect("cancelled pending settlement");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_commit_run_and_shutdown_cancellation_win_before_dispatch() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");

        let cancellation = AgentProviderCancellation::new();
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(input, &mut policy, &credential, cancellation.clone())
            .expect("admission");
        cancellation.cancel();
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Cancelled
        ));
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("sticky cancellation must settle immediately")
        };
        let receipt = settlement
            .settle(&mut policy)
            .expect("run cancellation settlement");
        assert_eq!(receipt.usage_accounting(), AgentModelUsageAccounting::Exact);
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(receipt.output_tokens(), 0);
        assert_eq!(receipt.cost_micro_usd(), 0);

        let (mut shutdown_policy, shutdown_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                shutdown_input,
                &mut shutdown_policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("shutdown admission");
        transport.seal();
        let result = attempt
            .execute(|_| AgentProviderBatchDisposition::Continue)
            .await;
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Cancelled
        ));
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("sticky shutdown must settle immediately")
        };
        settlement
            .settle(&mut shutdown_policy)
            .expect("shutdown cancellation settlement");
        let snapshot = transport.snapshot().expect("snapshot");
        assert!(snapshot.is_sealed());
        assert!(snapshot.is_quiescent());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn post_dispatch_cancellation_consumes_the_reservation_ceiling() {
        let server = StalledResponseServer::spawn();
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::try_new(
                Duration::from_secs(5),
                Duration::from_secs(2),
                Duration::from_secs(2),
            )
            .expect("config"),
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let cancellation = AgentProviderCancellation::new();
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(input, &mut policy, &credential, cancellation.clone())
            .expect("admission");
        let task = tokio::spawn(async move {
            attempt
                .execute(|_| AgentProviderBatchDisposition::Continue)
                .await
        });

        server.wait_until_request();
        cancellation.cancel();
        let result = task.await.expect("transport task");
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Cancelled
        ));
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("ambiguous cancellation must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        assert_eq!(settlement.settlement(), AgentModelCallSettlement::Cancelled);
        let receipt = settlement
            .settle(&mut policy)
            .expect("conservative cancellation settlement");
        assert_eq!(receipt.input_tokens(), 18);
        assert_eq!(receipt.output_tokens(), 20);
        assert_eq!(receipt.cost_micro_usd(), 100);
        assert!(transport.snapshot().expect("snapshot").is_idle());

        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head).expect("request head");
        assert!(head.starts_with("POST /v1/responses HTTP/1.1\r\n"));
    }

    #[test]
    fn abort_before_a_drive_settles_exact_zero_and_releases_the_slot() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        {
            let unpolled =
                attempt.execute_openai_exact_local(|_| AgentProviderBatchDisposition::Continue);
            drop(unpolled);
        }
        transport.seal();
        let result = attempt
            .abort(AgentProviderAbortReason::HostDeadline)
            .expect("ready operation abort");
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::NotDispatched
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("undispatched abort must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::Exact
        );
        let receipt = settlement
            .settle(&mut policy)
            .expect("exact abort settlement");
        assert_eq!(receipt.input_tokens(), 0);
        assert!(transport
            .try_prove_shutdown()
            .expect("abort releases sealed transport")
            .snapshot()
            .is_quiescent());
    }

    #[test]
    fn terminal_operation_shell_redacts_and_releases_credential_material() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let result = attempt
            .cancel_without_dispatch()
            .expect("terminal cancellation");
        let shell_debug = format!("{attempt:?}");
        assert!(!shell_debug.contains("synthetic-openai-key"));
        assert!(attempt.credential.is_none());
        assert_eq!(attempt.call(), None);
        assert_eq!(attempt.input_evidence(), None);
        assert_eq!(attempt.input_metrics(), None);
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("undispatched cancellation must settle immediately")
        };
        settlement.settle(&mut policy).expect("terminal settlement");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_count_drive_can_only_abort_with_exact_zero_accounting() {
        let server = StalledResponseServer::spawn();
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_exact_fixture();
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        {
            let drive = attempt.count_openai_input_tokens();
            tokio::pin!(drive);
            let observation_deadline = Instant::now()
                .checked_add(Duration::from_secs(1))
                .expect("observation deadline");
            while !server.request_observed() {
                assert!(
                    Instant::now() < observation_deadline,
                    "count request not observed"
                );
                tokio::select! {
                    result = &mut drive => panic!("stalled count completed: {result:?}"),
                    () = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
        }
        assert!(matches!(
            attempt.count_openai_input_tokens().await,
            AgentProviderExactCountOutcome::Unavailable(
                AgentProviderAttemptStateError::CountRecoveryRequired
            )
        ));
        transport.seal();
        let result = attempt
            .abort(AgentProviderAbortReason::HostDeadline)
            .expect("dropped count abort");
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::InputTokenCountDisclosed
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let receipt_before_settlement = result.input_metric_receipt();
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("count abort must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::Exact
        );
        let receipt = settlement
            .settle(&mut policy)
            .expect("exact count abort settlement");
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(receipt_before_settlement.call(), receipt.id());
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport
            .try_prove_shutdown()
            .expect("count abort releases slot")
            .snapshot()
            .is_quiescent());
        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head).expect("count request head");
        assert!(head.starts_with("POST /v1/responses/input_tokens HTTP/1.1\r\n"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_generation_drive_aborts_at_reservation_ceiling() {
        let server = StalledResponseServer::spawn();
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        {
            let drive =
                attempt.execute_openai_exact_local(|_| AgentProviderBatchDisposition::Continue);
            tokio::pin!(drive);
            let observation_deadline = Instant::now()
                .checked_add(Duration::from_secs(1))
                .expect("observation deadline");
            while !server.request_observed() {
                assert!(
                    Instant::now() < observation_deadline,
                    "generation request not observed"
                );
                tokio::select! {
                    result = &mut drive => panic!("stalled generation completed: {result:?}"),
                    () = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
        }
        assert_eq!(
            attempt
                .execute_openai_exact_local(|_| AgentProviderBatchDisposition::Continue)
                .await
                .expect_err("dropped drive must be abort-only"),
            AgentProviderAttemptStateError::GenerationRecoveryRequired
        );
        transport.seal();
        let result = attempt
            .abort(AgentProviderAbortReason::HostDeadline)
            .expect("dropped generation abort");
        assert_eq!(
            result.disclosure_stage(),
            AgentProviderDisclosureStage::ModelRequestMayHaveDispatched
        );
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("generation abort must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        let receipt = settlement
            .settle(&mut policy)
            .expect("ceiling generation abort settlement");
        assert_eq!(receipt.input_tokens(), 18);
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport
            .try_prove_shutdown()
            .expect("generation abort releases slot")
            .snapshot()
            .is_quiescent());
        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head).expect("generation request head");
        assert!(head.starts_with("POST /v1/responses HTTP/1.1\r\n"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_generation_drive_before_eof_aborts_at_reservation_ceiling() {
        let server = HeldEofServer::spawn(openai_success_stream());
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            server.openai.as_str(),
            server.anthropic.as_str(),
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        {
            let drive =
                attempt.execute_openai_exact_local(|_| AgentProviderBatchDisposition::Continue);
            tokio::pin!(drive);
            let observation_deadline = Instant::now()
                .checked_add(Duration::from_secs(1))
                .expect("observation deadline");
            while !server.terminal_observed() {
                assert!(
                    Instant::now() < observation_deadline,
                    "terminal body was not observed"
                );
                tokio::select! {
                    result = &mut drive => panic!("held EOF drive completed: {result:?}"),
                    () = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
        }
        transport.seal();
        let result = attempt
            .abort(AgentProviderAbortReason::HostDeadline)
            .expect("held EOF abort");
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("held EOF abort must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        let receipt = settlement
            .settle(&mut policy)
            .expect("held EOF ceiling settlement");
        assert_eq!(receipt.input_tokens(), 18);
        assert!(transport
            .try_prove_shutdown()
            .expect("held EOF abort releases slot")
            .snapshot()
            .is_quiescent());
        server.release_eof();
        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head).expect("generation request head");
        assert!(head.starts_with("POST /v1/responses HTTP/1.1\r\n"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn counted_operation_abort_preserves_exact_input_receipt_without_generation() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "application/json"),
                ("Content-Encoding", "identity"),
            ],
            vec![br#"{"object":"response.input_tokens","input_tokens":17}"#.to_vec()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_exact_fixture();
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let counted = match attempt.count_openai_input_tokens().await {
            AgentProviderExactCountOutcome::Counted(counted) => counted,
            AgentProviderExactCountOutcome::Failed(result) => {
                panic!("count terminalized unexpectedly: {result:?}")
            }
            AgentProviderExactCountOutcome::Unavailable(error) => {
                panic!("count unavailable: {error:?}")
            }
        };
        let counted_receipt = counted.input_metric_receipt();
        drop(counted);
        transport.seal();
        let result = attempt
            .abort(AgentProviderAbortReason::ControllerFault)
            .expect("counted abort");
        assert_eq!(result.input_metric_receipt(), counted_receipt);
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::ExactZeroBeforeModelDispatch
        );
        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("counted abort must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::Exact
        );
        let receipt = settlement
            .settle(&mut policy)
            .expect("counted abort settlement");
        assert_eq!(receipt.id(), counted_receipt.call());
        assert_eq!(receipt.input_tokens(), 0);
        assert_eq!(policy.pending_model_calls(), 0);
        assert!(transport
            .try_prove_shutdown()
            .expect("counted abort releases slot")
            .snapshot()
            .is_quiescent());
        let captured = server.finish();
        let head = std::str::from_utf8(&captured.head).expect("count request head");
        assert!(head.starts_with("POST /v1/responses/input_tokens HTTP/1.1\r\n"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panicking_batch_consumer_returns_authority_and_fail_stops_transport() {
        let server = OneShotServer::spawn(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream; charset=utf-8"),
                ("Content-Encoding", "identity"),
            ],
            vec![openai_success_stream()],
        );
        let transport = test_transport(&server);
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let mut attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        let mut consumed = false;
        let result = attempt
            .execute(|batch| {
                consumed = true;
                assert!(!batch.deltas().is_empty());
                panic!("synthetic provider batch consumer panic");
            })
            .await;
        assert!(consumed);
        assert_eq!(
            result.usage_knowledge(),
            AgentProviderUsageKnowledge::UnknownAfterDispatch
        );
        assert!(matches!(
            result.outcome(),
            AgentProviderTransportOutcome::Failed(failure)
                if failure.class() == AgentProviderFailureClass::Integration
                    && failure.retry_disposition()
                        == AgentProviderRetryDisposition::Never
        ));
        let snapshot = transport.snapshot().expect("snapshot");
        assert!(snapshot.is_sealed());
        assert!(snapshot.is_quiescent());

        let (mut refused_policy, refused_input) =
            provider_fixture(AgentProviderKind::OpenAiResponses);
        assert!(matches!(
            transport.try_admit(
                refused_input,
                &mut refused_policy,
                &credential,
                AgentProviderCancellation::new(),
            ),
            Err(AgentProviderAdmissionError::Sealed)
        ));
        assert_eq!(refused_policy.pending_model_calls(), 0);

        let AgentProviderPolicySettlement::Immediate(settlement) = result.into_policy_settlement()
        else {
            panic!("integration panic must settle immediately")
        };
        assert_eq!(
            settlement.usage_accounting(),
            AgentModelUsageAccounting::ReservationCeiling
        );
        assert_eq!(
            settlement.settlement(),
            AgentModelCallSettlement::ProviderFailed
        );
        let receipt = settlement
            .settle(&mut policy)
            .expect("conservative integration settlement");
        assert_eq!(receipt.input_tokens(), 18);
        assert_eq!(receipt.output_tokens(), 20);
        assert_eq!(receipt.cost_micro_usd(), 100);
        assert_eq!(policy.pending_model_calls(), 0);
        let _captured = server.finish();
    }

    #[test]
    fn abandoned_committed_attempt_fail_stops_the_shared_transport() {
        let transport = AgentProviderTransport::try_new_loopback(
            AgentProviderTransportConfig::STANDARD,
            "http://127.0.0.1:9/v1/responses",
            "http://127.0.0.1:9/v1/messages",
        )
        .expect("transport");
        let credential = AgentProviderCredential::try_new(
            AgentProviderKind::OpenAiResponses,
            "synthetic-openai-key".to_owned(),
        )
        .expect("credential");
        let (mut policy, input) = provider_fixture(AgentProviderKind::OpenAiResponses);
        let attempt = transport
            .try_admit(
                input,
                &mut policy,
                &credential,
                AgentProviderCancellation::new(),
            )
            .expect("admission");
        drop(attempt);
        let snapshot = transport.snapshot().expect("snapshot");
        assert!(snapshot.is_sealed());
        assert!(snapshot.is_quiescent());
        assert_eq!(policy.pending_model_calls(), 1);
    }

    #[test]
    fn retry_after_and_status_mapping_never_grant_retry_authority() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("3"));
        let rate_limit = status_failure(StatusCode::TOO_MANY_REQUESTS, &headers);
        assert_eq!(rate_limit.class(), AgentProviderFailureClass::RateLimited);
        assert_eq!(
            rate_limit
                .retry_after()
                .map(AgentProviderRetryAfter::millis),
            Some(3_000)
        );
        assert_eq!(
            rate_limit.retry_disposition(),
            AgentProviderRetryDisposition::PolicyMayRetry
        );
        let auth = status_failure(StatusCode::UNAUTHORIZED, &headers);
        assert_eq!(auth.class(), AgentProviderFailureClass::Authentication);
        assert_eq!(auth.retry_after(), None);
        assert_eq!(
            auth.retry_disposition(),
            AgentProviderRetryDisposition::Never
        );
    }

    #[test]
    fn request_reader_uses_a_bounded_exact_body() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let address = listener.local_addr().expect("address");
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            read_request(&mut stream).expect("request")
        });
        let mut stream = TcpStream::connect(address).expect("connect");
        stream
            .write_all(b"POST / HTTP/1.1\r\nContent-Length: 4\r\n\r\ntest")
            .expect("write");
        let captured = thread.join().expect("join");
        assert_eq!(captured.body.len(), 4);
    }

    #[test]
    fn cancellation_wait_registration_has_no_lost_wakeup() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let cancellation = AgentProviderCancellation::new();
            cancellation.cancel();
            tokio::time::timeout(Duration::from_millis(50), cancellation.cancelled())
                .await
                .expect("sticky cancellation");
        });
    }
}
