//! Account-authorized ChatGPT models and profile-scoped choices.
use super::*;
use zephium_core::ids::ProfileId;
use zephium_core::work::model::{
    WorkModelEntry, WorkModelProvider, WorkModelReasoning, WorkModelRole,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkKeyStateV1 {
    Missing,
    Set,
    Valid,
    Invalid,
}

/// Why the last action did not do what was asked. Closed; no provider text.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkModelsFaultV1 {
    /// The provider refused the key.
    KeyRefused,
    /// The provider could not be reached.
    Unreachable,
    Billing,
    RateLimited,
    ProviderDown,
    Request,
    /// The Keychain refused.
    Keychain,
    /// The Keychain is waiting for the person to answer its prompt.
    KeychainWaiting,
    /// The request was malformed (an unknown model, a bad address).
    Invalid,
    /// Models are unavailable in this build or profile.
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkProviderStatusV1 {
    pub(crate) provider: WorkModelProvider,
    pub(crate) key: WorkKeyStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) fault: Option<WorkModelsFaultV1>,
    /// The OpenAI-compatible endpoint's base URL.
    pub(crate) base: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkModelRolesV1 {
    pub(crate) lead: Option<String>,
    pub(crate) page: Option<String>,
    pub(crate) light: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkCloudStatusV1 {
    pub(crate) signed_in: bool,
    pub(crate) plan: Option<String>,
    /// Hosted entries listed for this account but locked by its plan: shown
    /// to explain the plan, never choosable.
    pub(crate) locked: Vec<String>,
    /// The cheapest plan each locked entry needs, when the server states it.
    pub(crate) required: Vec<WorkLockedPlanV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkLockedPlanV1 {
    pub(crate) id: String,
    pub(crate) plan: Option<String>,
}

/// The efforts one entry takes as the lead, weakest first.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkModelEffortsV1 {
    pub(crate) id: String,
    pub(crate) levels: Vec<WorkModelReasoning>,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkModelsV1 {
    pub(crate) version: u32,
    pub(crate) profile: String,
    pub(crate) entries: Vec<WorkModelEntry>,
    /// What the person chose per role.
    pub(crate) chosen: WorkModelRolesV1,
    /// What each role runs with now; null when no provider is usable.
    pub(crate) effective: WorkModelRolesV1,
    pub(crate) providers: Vec<WorkProviderStatusV1>,
    pub(crate) cloud: WorkCloudStatusV1,
    pub(crate) fault: Option<WorkModelsFaultV1>,
    /// Entries that take an effort; the rest take none.
    pub(crate) efforts: Vec<WorkModelEffortsV1>,
    /// The effort the lead runs at now; null when its model takes none.
    pub(crate) effort: Option<WorkModelReasoning>,
}

impl WorkModelsV1 {
    fn failed(profile: String, fault: WorkModelsFaultV1) -> Self {
        Self {
            version: 1,
            profile,
            entries: Vec::new(),
            chosen: WorkModelRolesV1::default(),
            effective: WorkModelRolesV1::default(),
            providers: Vec::new(),
            cloud: WorkCloudStatusV1::default(),
            fault: Some(fault),
            efforts: Vec::new(),
            effort: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkMoreModelsV1 {
    pub(crate) provider: WorkModelProvider,
    pub(crate) entries: Vec<WorkModelEntry>,
    pub(crate) fault: Option<WorkModelsFaultV1>,
}

/// Something in the picker or Settings → AI changed; read it again.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[tauri_specta(event_name = "zephium:work-models-changed")]
pub(crate) struct WorkModelsChanged {
    pub(crate) version: u32,
}

#[cfg(feature = "work-product")]
mod product {
    use super::*;
    use zephium_app::work_models as models;
    use zephium_core::work::model::WorkModelError;

    struct AppSettings;

    impl models::WorkModelSettings for AppSettings {
        fn get(&self, key: &str) -> Option<String> {
            use zephium_core::ports::store::Store;
            APP_STORE.get().and_then(|store| store.app_setting(key))
        }
        fn set(&self, key: String, value: String) -> bool {
            use zephium_core::ports::store::Store;
            APP_STORE
                .get()
                .is_some_and(|store| store.set_app_setting(key, value))
        }
    }

    pub(super) fn install(app: &tauri::AppHandle) {
        models::install_settings(std::sync::Arc::new(AppSettings));
        let app = app.clone();
        models::set_observer(Box::new(move || {
            emit_to_privileged(
                &app,
                MAIN_LABEL,
                "zephium:work-models-changed",
                &WorkModelsChanged { version: 1 },
            );
            super::super::account::emit_changed(&app);
        }));
    }

    fn key(status: models::WorkKeyStatus) -> WorkKeyStateV1 {
        match status {
            models::WorkKeyStatus::Missing => WorkKeyStateV1::Missing,
            models::WorkKeyStatus::Set => WorkKeyStateV1::Set,
            models::WorkKeyStatus::Valid => WorkKeyStateV1::Valid,
            models::WorkKeyStatus::Invalid => WorkKeyStateV1::Invalid,
        }
    }

    fn provider_fault(fault: models::WorkKeyFault) -> WorkModelsFaultV1 {
        match fault {
            models::WorkKeyFault::WrongKey => WorkModelsFaultV1::KeyRefused,
            models::WorkKeyFault::Billing => WorkModelsFaultV1::Billing,
            models::WorkKeyFault::RateLimited => WorkModelsFaultV1::RateLimited,
            models::WorkKeyFault::ProviderDown => WorkModelsFaultV1::ProviderDown,
            models::WorkKeyFault::Offline => WorkModelsFaultV1::Unreachable,
            models::WorkKeyFault::Keychain => WorkModelsFaultV1::Keychain,
            models::WorkKeyFault::KeychainWaiting => WorkModelsFaultV1::KeychainWaiting,
            models::WorkKeyFault::Request => WorkModelsFaultV1::Request,
        }
    }

    pub(super) fn fault(error: WorkModelError) -> WorkModelsFaultV1 {
        match error {
            WorkModelError::Network => WorkModelsFaultV1::Unreachable,
            WorkModelError::Overloaded => WorkModelsFaultV1::ProviderDown,
            WorkModelError::RateLimited { .. } => WorkModelsFaultV1::RateLimited,
            WorkModelError::OverBudget => WorkModelsFaultV1::Billing,
            WorkModelError::Protocol => WorkModelsFaultV1::Request,
            WorkModelError::Unauthorized => WorkModelsFaultV1::Keychain,
            WorkModelError::MissingKey => WorkModelsFaultV1::KeyRefused,
            _ => WorkModelsFaultV1::Invalid,
        }
    }

    pub(super) async fn view(profile: ProfileId, fault: Option<WorkModelsFaultV1>) -> WorkModelsV1 {
        let view = models::view(profile).await;
        let [lead, page, light] = view.chosen;
        let chosen = WorkModelRolesV1 { lead, page, light };
        let [lead, page, light] = view.effective;
        let effective = WorkModelRolesV1 { lead, page, light };
        WorkModelsV1 {
            version: 1,
            profile: profile.to_string(),
            entries: view.entries,
            chosen,
            effective,
            providers: view
                .providers
                .into_iter()
                .map(|provider| WorkProviderStatusV1 {
                    provider: provider.provider,
                    key: key(provider.key),
                    fault: provider.fault.map(provider_fault),
                    base: provider.base,
                })
                .collect(),
            cloud: WorkCloudStatusV1 {
                signed_in: view.cloud_signed_in,
                plan: view.cloud_plan,
                locked: view.cloud_locked,
                required: view
                    .cloud_locked_plans
                    .into_iter()
                    .map(|(id, plan)| WorkLockedPlanV1 { id, plan })
                    .collect(),
            },
            fault,
            efforts: view
                .efforts
                .into_iter()
                .map(|(id, levels)| WorkModelEffortsV1 { id, levels })
                .collect(),
            effort: view.effort,
        }
    }

    pub(super) async fn act(profile: ProfileId, action: Action) -> Option<WorkModelsFaultV1> {
        let result = match action {
            Action::Read => {
                models::refresh_catalog().await;
                Ok(())
            }
            Action::Choose(role, id) => models::choose(profile, role, id.as_deref()),
            Action::Effort(effort) => models::set_effort(profile, effort),
        };
        result.err().map(fault)
    }

    pub(super) async fn more(
        provider: WorkModelProvider,
    ) -> Result<Vec<WorkModelEntry>, WorkModelsFaultV1> {
        models::more_models(provider)
            .await
            .map_err(|error| match error {
                WorkModelError::Unauthorized => WorkModelsFaultV1::KeyRefused,
                WorkModelError::MissingKey => WorkModelsFaultV1::KeyRefused,
                error => fault(error),
            })
    }
}

pub(crate) enum Action {
    Read,
    Choose(WorkModelRole, Option<String>),
    Effort(Option<WorkModelReasoning>),
}

pub(crate) fn install(app: &tauri::AppHandle) {
    #[cfg(feature = "work-product")]
    product::install(app);
    #[cfg(not(feature = "work-product"))]
    let _ = app;
}

fn action_name(action: &Action) -> &'static str {
    match action {
        Action::Read => "read",
        Action::Choose(..) => "choose",
        Action::Effort(..) => "effort",
    }
}

fn log(arguments: std::fmt::Arguments<'_>) {
    #[cfg(feature = "work-product")]
    super::work_provider::record_diagnostic(arguments);
    #[cfg(not(feature = "work-product"))]
    super::write_diagnostic(arguments);
}

/// Models are app-wide except choices, which belong to the Work profile the
/// caller names; the id must parse exactly.
fn profile_of(caller: &WebviewWindow, expected: &str, label: &str) -> Option<ProfileId> {
    if !authorize(caller, CallerPolicy::Main, label) {
        return None;
    }
    ProfileId::parse(expected).filter(|id| id.to_string() == expected)
}

async fn respond(
    caller: WebviewWindow,
    expected_profile: String,
    action: Action,
    label: &'static str,
) -> WorkModelsV1 {
    let Some(profile) = profile_of(&caller, &expected_profile, label) else {
        return WorkModelsV1::failed(expected_profile, WorkModelsFaultV1::Unavailable);
    };
    let name = action_name(&action);
    #[cfg(feature = "work-product")]
    {
        let fault = product::act(profile, action).await;
        log(format_args!(
            "work: phase=models action={name} fault={fault:?}"
        ));
        product::view(profile, fault).await
    }
    #[cfg(not(feature = "work-product"))]
    {
        let _ = (profile, action);
        log(format_args!(
            "work: phase=models action={name} fault=unavailable"
        ));
        WorkModelsV1::failed(expected_profile, WorkModelsFaultV1::Unavailable)
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn work_models(caller: WebviewWindow, expected_profile: String) -> WorkModelsV1 {
    respond(caller, expected_profile, Action::Read, "work_models").await
}

/// How a run would search the web with the models chosen now.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkSearchReadyV1 {
    /// The person's search engine's results page, read in the agent browser
    /// with no model; a model's search answers only when the page cannot be used.
    Browser,
    /// A chosen model's own provider search.
    Native,
    /// The fixed public search with the person's OpenAI key.
    OpenAi,
    #[default]
    Unavailable,
}

/// A narrow readiness contract for the Work composer. Only a missing lead blocks.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkModelsReadyV1 {
    pub(crate) version: u16,
    pub(crate) profile: String,
    pub(crate) ready: bool,
    pub(crate) missing_roles: Vec<WorkModelRole>,
    #[serde(default)]
    pub(crate) search: WorkSearchReadyV1,
    pub(crate) fault: Option<WorkModelsFaultV1>,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn work_models_ready(
    caller: WebviewWindow,
    expected_profile: String,
) -> WorkModelsReadyV1 {
    let profile = profile_of(&caller, &expected_profile, "work_models_ready");
    #[cfg(feature = "work-product")]
    let (ready, search) = match profile {
        Some(profile) => {
            use zephium_app::work_models::{self as models, WorkSearchReadiness};
            // Runs search results pages in the agent browser wherever Work
            // runs; a model's search is only their fallback.
            let search = if cfg!(any(target_os = "macos", target_os = "windows")) {
                WorkSearchReadyV1::Browser
            } else {
                match models::search_readiness(profile).await {
                    WorkSearchReadiness::Native(_) => WorkSearchReadyV1::Native,
                    WorkSearchReadiness::OpenAi => WorkSearchReadyV1::OpenAi,
                    WorkSearchReadiness::Unavailable => WorkSearchReadyV1::Unavailable,
                }
            };
            (models::ready(profile).await, search)
        }
        None => (false, WorkSearchReadyV1::Unavailable),
    };
    #[cfg(not(feature = "work-product"))]
    let (ready, search) = (false, WorkSearchReadyV1::Unavailable);
    WorkModelsReadyV1 {
        version: 1,
        profile: expected_profile,
        ready,
        missing_roles: if ready {
            vec![]
        } else {
            vec![WorkModelRole::Lead]
        },
        search,
        fault: (profile.is_none() || !cfg!(feature = "work-product"))
            .then_some(WorkModelsFaultV1::Unavailable),
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn work_choose_model(
    caller: WebviewWindow,
    expected_profile: String,
    role: WorkModelRole,
    id: Option<String>,
) -> WorkModelsV1 {
    respond(
        caller,
        expected_profile,
        Action::Choose(role, id),
        "work_choose_model",
    )
    .await
}

/// The lead's reasoning effort for this profile; null returns it to each model's default.
#[tauri::command]
#[specta::specta]
pub(crate) async fn work_set_model_effort(
    caller: WebviewWindow,
    expected_profile: String,
    effort: Option<WorkModelReasoning>,
) -> WorkModelsV1 {
    respond(
        caller,
        expected_profile,
        Action::Effort(effort),
        "work_set_model_effort",
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn work_more_models(
    caller: WebviewWindow,
    expected_profile: String,
    provider: WorkModelProvider,
) -> WorkMoreModelsV1 {
    let refused = |fault| WorkMoreModelsV1 {
        provider,
        entries: Vec::new(),
        fault: Some(fault),
    };
    if profile_of(&caller, &expected_profile, "work_more_models").is_none() {
        return refused(WorkModelsFaultV1::Unavailable);
    }
    #[cfg(feature = "work-product")]
    {
        match product::more(provider).await {
            Ok(entries) => {
                log(format_args!(
                    "work: phase=models action=more entries={}",
                    entries.len()
                ));
                WorkMoreModelsV1 {
                    provider,
                    entries,
                    fault: None,
                }
            }
            Err(fault) => {
                log(format_args!(
                    "work: phase=models action=more fault={fault:?}"
                ));
                refused(fault)
            }
        }
    }
    #[cfg(not(feature = "work-product"))]
    refused(WorkModelsFaultV1::Unavailable)
}

#[cfg(test)]
mod release_tests {
    use super::*;

    #[test]
    fn readiness_and_provider_faults_have_stable_wire_shapes() {
        let ready = WorkModelsReadyV1 {
            version: 1,
            profile: "profile".into(),
            ready: false,
            missing_roles: vec![WorkModelRole::Lead],
            search: WorkSearchReadyV1::Unavailable,
            fault: None,
        };
        assert_eq!(
            serde_json::to_value(ready).unwrap(),
            serde_json::json!({
                "version": 1, "profile": "profile", "ready": false,
                "missing_roles": ["lead"], "search": "unavailable", "fault": null
            })
        );
        let mut provider = WorkProviderStatusV1 {
            provider: WorkModelProvider::OpenAi,
            key: WorkKeyStateV1::Valid,
            fault: None,
            base: None,
        };
        assert!(serde_json::to_value(&provider)
            .unwrap()
            .get("fault")
            .is_none());
        provider.fault = Some(WorkModelsFaultV1::Billing);
        let value = serde_json::to_value(provider).unwrap();
        assert_eq!(value["key"], "valid");
        assert_eq!(value["fault"], "billing");
    }
}
