//! ChatGPT-only model composition. Old provider IDs remain readable in history;
//! no API key, hosted gateway, local endpoint or other provider is selectable.
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use zephium_agentic::lead::{
    models, LeadClient, LeadCredential, LeadSecret, LeadSecretFuture, LeadTarget,
};
use zephium_core::ids::ProfileId;
use zephium_core::work::model::*;

use WorkModelProvider as P;
const ROLES: [WorkModelRole; 3] = [
    WorkModelRole::Lead,
    WorkModelRole::Page,
    WorkModelRole::Light,
];
/// Compatibility projection: there are no user-configured provider keys.
pub const KEYED_PROVIDERS: [WorkModelProvider; 0] = [];

/// Native durable settings, shared with the browser's ordinary preferences.
pub trait WorkModelSettings: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: String, value: String) -> bool;
}

/// Local attribution of a model call, never sent as account or billing data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkCallSource {
    Task { profile: String, work: String },
    Voice,
    Page { conversation: String, reply: u32 },
}

static SETTINGS: RwLock<Option<Arc<dyn WorkModelSettings>>> = RwLock::new(None);
static CHATGPT: RwLock<Option<Arc<quasar_chatgpt::Runtime>>> = RwLock::new(None);
type Observer = Box<dyn Fn() + Send + Sync>;
static OBSERVER: OnceLock<Observer> = OnceLock::new();

#[derive(Default)]
struct Catalog {
    epoch: Option<u64>,
    fetched: Option<Instant>,
    entries: Vec<WorkModelEntry>,
    fault: Option<WorkKeyFault>,
}
static CATALOG: Mutex<Catalog> = Mutex::new(Catalog {
    epoch: None,
    fetched: None,
    entries: Vec::new(),
    fault: None,
});
static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn install_settings(settings: Arc<dyn WorkModelSettings>) {
    *SETTINGS.write().unwrap_or_else(|p| p.into_inner()) = Some(settings);
}

/// Only the trusted account owner installs a ChatGPT runtime.
pub fn install_chatgpt(runtime: Arc<quasar_chatgpt::Runtime>) {
    *CHATGPT.write().unwrap_or_else(|p| p.into_inner()) = Some(runtime);
    session_changed();
}

pub fn set_observer(observer: Observer) {
    let _ = OBSERVER.set(observer);
}
fn notify() {
    if let Some(observer) = OBSERVER.get() {
        observer();
    }
}
fn catalog() -> std::sync::MutexGuard<'static, Catalog> {
    CATALOG.lock().unwrap_or_else(|p| p.into_inner())
}
fn runtime() -> Option<Arc<quasar_chatgpt::Runtime>> {
    CHATGPT.read().ok()?.clone()
}

pub fn session_changed() {
    *catalog() = Catalog::default();
    notify();
}

pub(crate) fn setting(key: &str) -> Option<String> {
    SETTINGS
        .read()
        .ok()?
        .as_ref()?
        .get(key)
        .filter(|v| !v.is_empty())
}
pub(crate) fn store_setting(key: String, value: String) -> bool {
    SETTINGS
        .read()
        .ok()
        .and_then(|s| s.clone())
        .is_some_and(|s| s.set(key, value))
}

/// Legacy wire states now project connection readiness, never a stored API key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkKeyStatus {
    Missing,
    Set,
    Valid,
    Invalid,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkKeyFault {
    WrongKey,
    Billing,
    RateLimited,
    ProviderDown,
    Offline,
    Keychain,
    KeychainWaiting,
    Request,
}

fn error(error: quasar_chatgpt::Error) -> WorkModelError {
    match error {
        quasar_chatgpt::Error::Network => WorkModelError::Network,
        quasar_chatgpt::Error::Cancelled => WorkModelError::Cancelled,
        quasar_chatgpt::Error::RateLimited
        | quasar_chatgpt::Error::UsageLimit
        | quasar_chatgpt::Error::RetryLater => WorkModelError::RateLimited {
            retry_after_ms: None,
        },
        quasar_chatgpt::Error::SignedOut | quasar_chatgpt::Error::PlanPermissionRequired => {
            WorkModelError::MissingKey
        }
        _ => WorkModelError::Unauthorized,
    }
}

fn catalog_entry(model: quasar_chatgpt::Model, index: usize) -> WorkModelEntry {
    let known = models::builtin()
        .into_iter()
        .find(|entry| entry.model.provider == P::OpenAi && entry.model.model == model.slug);
    let mut entry = known.unwrap_or_else(|| WorkModelEntry {
        id: String::new(),
        model: WorkModelRef {
            provider: P::OpenAi,
            wire: WorkModelWire::OpenAiResponses,
            model: model.slug.clone(),
        },
        display_name: String::new(),
        roles: ROLES.to_vec(),
        recommended: false,
        context_window: 128_000,
        max_output: 8192,
        supports: WorkModelSupports {
            tools: true,
            vision: false,
            prompt_cache: true,
            reasoning: false,
            native_search: false,
        },
        price: None,
    });
    entry.id = models::entry_id(P::OpenAi, &model.slug);
    entry.display_name = model.display_name;
    entry.roles = ROLES.to_vec();
    entry.recommended = index == 0;
    // API dollar prices do not describe usage through a ChatGPT plan.
    entry.price = None;
    entry
}

pub async fn refresh_catalog() {
    update_catalog(false).await;
}

async fn update_catalog(force: bool) {
    let _refresh = REFRESH.lock().await;
    let Some(runtime) = runtime() else {
        return;
    };
    let epoch = runtime.epoch();
    let fresh = {
        let state = catalog();
        state.epoch == Some(epoch)
            && state
                .fetched
                .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
    };
    if fresh && !force {
        return;
    }
    let session = runtime.session().await;
    let epoch = runtime.epoch();
    // A signed-in session may have a quarantined refresh to recover. Native
    // credential verification still gates the actual catalog request.
    if session.as_ref().is_ok_and(|s| {
        !s.active().is_some_and(|p| p.signed_in)
            || s.fault == Some(quasar_chatgpt::Error::PlanPermissionRequired)
    }) {
        *catalog() = Catalog::default();
        return;
    }
    let result = match session {
        Ok(_) => runtime.models().await,
        Err(error) => Err(error),
    };
    if runtime.epoch() != epoch {
        *catalog() = Catalog::default();
        notify();
        return;
    }
    let mut state = catalog();
    state.epoch = Some(epoch);
    state.fetched = Some(Instant::now());
    match result {
        Ok(models) => {
            state.entries = models
                .into_iter()
                .enumerate()
                .map(|(i, m)| catalog_entry(m, i))
                .collect();
            state.fault = None;
        }
        Err(error) => {
            state.entries.clear();
            state.fault = Some(match error {
                quasar_chatgpt::Error::Network => WorkKeyFault::Offline,
                quasar_chatgpt::Error::UsageLimit
                | quasar_chatgpt::Error::RateLimited
                | quasar_chatgpt::Error::RetryLater => WorkKeyFault::RateLimited,
                quasar_chatgpt::Error::Storage => WorkKeyFault::Keychain,
                _ => WorkKeyFault::WrongKey,
            });
        }
    }
    drop(state);
    notify();
}

fn choice_key(profile: ProfileId, role: WorkModelRole) -> String {
    let role = match role {
        WorkModelRole::Lead => "lead",
        WorkModelRole::Page => "page",
        WorkModelRole::Light => "light",
        WorkModelRole::Decision => "decision",
    };
    format!("work.models.{profile}.{role}")
}

fn chosen(
    entries: &[WorkModelEntry],
    profile: ProfileId,
    role: WorkModelRole,
) -> Option<WorkModelEntry> {
    let stored: WorkModelEntry =
        serde_json::from_str(&setting(&choice_key(profile, role))?).ok()?;
    if stored.model.provider != P::OpenAi || stored.model.wire != WorkModelWire::OpenAiResponses {
        return None;
    }
    // Re-resolve metadata from this account, never from an old persisted entry.
    entries
        .iter()
        .find(|e| e.model.model == stored.model.model)
        .cloned()
}
fn effective(
    entries: &[WorkModelEntry],
    profile: ProfileId,
    role: WorkModelRole,
) -> Option<WorkModelEntry> {
    chosen(entries, profile, role).or_else(|| entries.first().cloned())
}
fn efforts(entry: &WorkModelEntry) -> Vec<WorkModelReasoning> {
    if entry.supports.reasoning {
        models::efforts(P::OpenAi, &entry.model.model)
    } else {
        Vec::new()
    }
}
fn effort_key(profile: ProfileId) -> String {
    format!("work.models.{profile}.lead.effort")
}

pub fn lead_effort(profile: ProfileId, entry: &WorkModelEntry) -> Option<WorkModelReasoning> {
    let offered = efforts(entry);
    let stored =
        setting(&effort_key(profile)).and_then(|v| serde_json::from_str(&format!("\"{v}\"")).ok());
    stored.filter(|v| offered.contains(v)).or_else(|| {
        offered
            .contains(&WorkModelReasoning::Medium)
            .then_some(WorkModelReasoning::Medium)
    })
}
pub fn set_effort(
    profile: ProfileId,
    effort: Option<WorkModelReasoning>,
) -> Result<(), WorkModelError> {
    let value = effort
        .map(|v| {
            serde_json::to_string(&v)
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned()
        })
        .unwrap_or_default();
    if !store_setting(effort_key(profile), value) {
        return Err(WorkModelError::Unauthorized);
    }
    notify();
    Ok(())
}

struct Credential {
    runtime: Arc<quasar_chatgpt::Runtime>,
    epoch: u64,
}
impl LeadCredential for Credential {
    fn secret(&self) -> LeadSecretFuture<'_> {
        Box::pin(async move {
            let token = self.runtime.access_token(self.epoch).await.map_err(error)?;
            LeadSecret::new(token.expose().to_owned()).map(Arc::new)
        })
    }
    fn rejected(&self) {
        if self.runtime.epoch() == self.epoch {
            self.runtime.rejected();
        }
    }
}
struct ScopedClient {
    inner: LeadClient,
    runtime: Arc<quasar_chatgpt::Runtime>,
    epoch: u64,
    model: WorkModelRef,
}
impl WorkModelClient for ScopedClient {
    fn call<'a>(
        &'a self,
        request: WorkModelRequest,
        events: &'a (dyn Fn(WorkModelEvent) + Send + Sync),
    ) -> WorkModelFuture<'a> {
        Box::pin(async move {
            let mut changed = self.runtime.subscribe();
            if self.runtime.epoch() != self.epoch || request.model != self.model {
                return Err(WorkModelError::Unauthorized);
            }
            tokio::select! {
                biased;
                _ = changed.changed() => Err(WorkModelError::Cancelled),
                result = self.inner.call(request, events) => result,
            }
        })
    }
}

pub async fn resolve(
    profile: ProfileId,
    role: WorkModelRole,
) -> Result<(WorkModelRef, Arc<dyn WorkModelClient>), WorkModelError> {
    let (entry, client) = resolve_entry(profile, role).await?;
    Ok((entry.model, client))
}
pub async fn resolve_entry(
    profile: ProfileId,
    role: WorkModelRole,
) -> Result<(WorkModelEntry, Arc<dyn WorkModelClient>), WorkModelError> {
    resolve_entry_for(profile, role, None).await
}
pub async fn resolve_entry_for(
    profile: ProfileId,
    role: WorkModelRole,
    _source: Option<WorkCallSource>,
) -> Result<(WorkModelEntry, Arc<dyn WorkModelClient>), WorkModelError> {
    refresh_catalog().await;
    let runtime = runtime().ok_or(WorkModelError::MissingKey)?;
    let epoch = runtime.epoch();
    let state = catalog();
    if state.epoch != Some(epoch) {
        return Err(WorkModelError::MissingKey);
    }
    let entry = effective(&state.entries, profile, role).ok_or(WorkModelError::MissingKey)?;
    let credential = Arc::new(Credential {
        runtime: runtime.clone(),
        epoch,
    });
    let inner = LeadClient::new(LeadTarget::direct(P::OpenAi)?, credential, None);
    let client = ScopedClient {
        inner,
        runtime,
        epoch,
        model: entry.model.clone(),
    };
    Ok((entry, Arc::new(client)))
}

#[derive(Clone, Debug)]
pub struct WorkProviderView {
    pub provider: P,
    pub key: WorkKeyStatus,
    pub fault: Option<WorkKeyFault>,
    pub base: Option<String>,
}
#[derive(Clone, Debug)]
pub struct WorkModelsView {
    pub entries: Vec<WorkModelEntry>,
    pub chosen: [Option<String>; 3],
    pub effective: [Option<String>; 3],
    pub providers: Vec<WorkProviderView>,
    pub cloud_signed_in: bool,
    pub cloud_plan: Option<String>,
    pub cloud_locked: Vec<String>,
    pub cloud_locked_plans: Vec<(String, Option<String>)>,
    pub efforts: Vec<(String, Vec<WorkModelReasoning>)>,
    pub effort: Option<WorkModelReasoning>,
}
pub async fn view(profile: ProfileId) -> WorkModelsView {
    refresh_catalog().await;
    let state = catalog();
    let entries = state.entries.clone();
    WorkModelsView {
        chosen: ROLES.map(|r| chosen(&entries, profile, r).map(|e| e.id)),
        effective: ROLES.map(|r| effective(&entries, profile, r).map(|e| e.id)),
        effort: effective(&entries, profile, WorkModelRole::Lead)
            .and_then(|e| lead_effort(profile, &e)),
        efforts: entries
            .iter()
            .map(|e| (e.id.clone(), efforts(e)))
            .filter(|(_, v)| !v.is_empty())
            .collect(),
        providers: vec![WorkProviderView {
            provider: P::OpenAi,
            key: if entries.is_empty() {
                WorkKeyStatus::Missing
            } else {
                WorkKeyStatus::Valid
            },
            fault: state.fault,
            base: None,
        }],
        entries,
        cloud_signed_in: false,
        cloud_plan: None,
        cloud_locked: Vec::new(),
        cloud_locked_plans: Vec::new(),
    }
}
pub fn provider_failure(provider: P) -> Option<WorkKeyFault> {
    (provider == P::OpenAi).then(|| catalog().fault).flatten()
}
pub fn choose(
    profile: ProfileId,
    role: WorkModelRole,
    id: Option<&str>,
) -> Result<(), WorkModelError> {
    if !ROLES.contains(&role) {
        return Err(WorkModelError::BadRequest);
    }
    let value = match id {
        None => String::new(),
        Some(id) => serde_json::to_string(
            catalog()
                .entries
                .iter()
                .find(|e| e.id == id)
                .ok_or(WorkModelError::BadRequest)?,
        )
        .map_err(|_| WorkModelError::BadRequest)?,
    };
    if !store_setting(choice_key(profile, role), value) {
        return Err(WorkModelError::Unauthorized);
    }
    notify();
    Ok(())
}
pub async fn ready(profile: ProfileId) -> bool {
    refresh_catalog().await;
    effective(&catalog().entries, profile, WorkModelRole::Lead).is_some()
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkSearchReadiness {
    Native(WorkModelEntry),
    OpenAi,
    Unavailable,
}
pub async fn search_readiness(profile: ProfileId) -> WorkSearchReadiness {
    refresh_catalog().await;
    effective(&catalog().entries, profile, WorkModelRole::Light)
        .filter(|e| e.supports.native_search)
        .map(WorkSearchReadiness::Native)
        .unwrap_or(WorkSearchReadiness::Unavailable)
}

// Old IPC clients cannot reintroduce an API key, provider or endpoint.
pub async fn set_key(_: P, _: String) -> Result<WorkKeyStatus, WorkModelError> {
    Err(WorkModelError::BadRequest)
}
pub async fn test_key(_: P) -> Result<WorkKeyStatus, WorkModelError> {
    Err(WorkModelError::BadRequest)
}
pub async fn clear_key(_: P) -> Result<(), WorkModelError> {
    Err(WorkModelError::BadRequest)
}
pub fn fill_defaults(_: ProfileId, _: P) {}
pub async fn set_compatible_base(_: Option<&str>) -> Result<(), WorkModelError> {
    Err(WorkModelError::BadRequest)
}
pub async fn more_models(provider: P) -> Result<Vec<WorkModelEntry>, WorkModelError> {
    if provider != P::OpenAi {
        return Err(WorkModelError::BadRequest);
    }
    update_catalog(true).await;
    Ok(catalog().entries.clone())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn legacy_provider_keys_and_endpoints_cannot_be_enabled() {
        for provider in [P::OpenAi, P::Anthropic, P::Google, P::Cloud, P::Compatible] {
            assert!(set_key(provider, "test-key".into()).await.is_err());
            assert!(test_key(provider).await.is_err());
        }
        assert!(set_compatible_base(Some("http://localhost:11434/v1"))
            .await
            .is_err());
        assert!(KEYED_PROVIDERS.is_empty());
    }
    #[test]
    fn account_catalog_has_only_openai_responses_and_no_api_dollar_prices() {
        let entry = catalog_entry(
            quasar_chatgpt::Model {
                slug: "account-model".into(),
                display_name: "Account model".into(),
            },
            0,
        );
        assert_eq!(entry.model.provider, P::OpenAi);
        assert_eq!(entry.model.wire, WorkModelWire::OpenAiResponses);
        assert!(entry.price.is_none());
        assert!(!entry.supports.vision);
    }
}
