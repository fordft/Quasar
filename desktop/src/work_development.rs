//! Release-excluded input/output adapter around ordinary Work admission.
//! No browser engine, policy bypass, tool loop or answer verifier lives here.

#[cfg(any(
    feature = "macos-work-navigation-probe",
    feature = "macos-work-rendering-probe"
))]
compile_error!("select the dynamic Work developer entry without a qualification launcher");

use serde::Deserialize;
use serde_json::json;
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use zephium_agent_controller::{
    AgentBrowserModel, AgentWorkAccountSource, AgentWorkEventKind, AgentWorkFailure,
    AgentWorkLocalActionPolicy,
};
use zephium_agent_provider_transport::{
    load_macos_probe_openai_credential, AgentProviderTransportConfig,
};
use zephium_agent_runtime::AgentRuntimeConfig;
use zephium_agentic::*;
use zephium_app::{
    AgentWorkApplicationConfig, AgentWorkProfileBinding, AgentWorkProfileReadiness,
    AgentWorkProfileRequest, RetainedWorkHandle, RetainedWorkPhase,
};
use zephium_work_composition::{
    PublicLocalActionWorkInvocation, PublicReadWorkAccount, PublicReadWorkInvocation,
    PublicReadWorkObjective, PublicReadWorkSettings,
};

const CONFIG_BYTES: u64 = 32 * 1024;
const OUTPUT_BYTES: usize = 64 * 1024;
const CONFIG_ENV: &str = "ZEPHIUM_WORK_REQUEST";
const FOREGROUND_GRACE: Duration = Duration::from_secs(10);
const REVIEW_ENV: &str = "ZEPHIUM_WORK_REVIEW";
const REVIEW_BYTES: u64 = 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewRequest {
    record: String,
    decision: ReviewDecision,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReviewDecision {
    AcceptFreshAdmission,
    Reject,
}

fn parse_review(
    bytes: &[u8],
) -> Result<(AgentWorkRecord, zephium_app::AgentWorkReviewDecision), &'static str> {
    if bytes.len() as u64 > REVIEW_BYTES {
        return Err("review exceeds byte limit");
    }
    let request: ReviewRequest =
        serde_json::from_slice(bytes).map_err(|_| "invalid review JSON")?;
    if request.record.len() != AGENT_WORK_RECORD_BYTES * 2 || !request.record.is_ascii() {
        return Err("review requires exact record bytes");
    }
    let mut record = [0; AGENT_WORK_RECORD_BYTES];
    for (index, byte) in record.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&request.record[index * 2..index * 2 + 2], 16)
            .map_err(|_| "invalid record encoding")?;
    }
    let record = AgentWorkRecord::decode(record)
        .filter(|record| record.disposition() == AgentWorkDisposition::Interrupted)
        .ok_or("review requires an interrupted record")?;
    Ok((
        record,
        match request.decision {
            ReviewDecision::AcceptFreshAdmission => {
                zephium_app::AgentWorkReviewDecision::AcceptFreshAdmission
            }
            ReviewDecision::Reject => zephium_app::AgentWorkReviewDecision::Reject,
        },
    ))
}

fn record_hex(record: AgentWorkRecord) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(AGENT_WORK_RECORD_BYTES * 2);
    for byte in record.as_bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    objective: String,
    account: Account,
    #[serde(default)]
    account_id: Option<String>,
    start_url: String,
    path_prefix: String,
    max_hops: usize,
    output_fields: Vec<Field>,
    model: Model,
    max_model_calls: u8,
    operations: u32,
    model_tokens: u64,
    cost_micro_usd: u64,
    deadline_seconds: u64,
    inspectable_public: bool,
    persist_result: bool,
    #[serde(default)]
    local_actions: Option<LocalActionApproval>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Field {
    name: String,
    max_bytes: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Model {
    Luna,
    Terra,
}

/// Explicit development assurance. Anonymous is product-equivalent. The actor
/// attestation exists only for a disposable qualification profile and is
/// intentionally weaker than production per-document identity evidence.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Account {
    AnonymousPublic,
    ActorAttestedDevelopment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalActionApproval {
    max_actions: u64,
    #[serde(default)]
    required_sequence: Option<RequiredLocalActionSequence>,
    #[serde(default)]
    clicks: Vec<ClickApproval>,
    #[serde(default)]
    fills: Vec<FillApproval>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RequiredLocalActionSequence {
    DialogSearchResult,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClickApproval {
    #[serde(default)]
    target_name: Option<String>,
    #[serde(default)]
    target_name_prefix: Option<String>,
    #[serde(default)]
    target_role: Option<ClickTargetRole>,
    #[serde(default)]
    effect: ClickEffect,
    verification: ClickVerification,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ClickVerification {
    TargetState { state: ClickState, present: bool },
    PageDialogOpened {},
    PageDialogClosed {},
}

impl From<ClickVerification> for SemanticVerification {
    fn from(value: ClickVerification) -> Self {
        match value {
            ClickVerification::TargetState { state, present } => Self::TargetState {
                state: state.into(),
                present,
            },
            ClickVerification::PageDialogOpened {} => Self::PageDialogOpened,
            ClickVerification::PageDialogClosed {} => Self::PageDialogClosed,
        }
    }
}

/// Trusted fixture classification, never inferred from the model or page.
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClickEffect {
    Read,
    #[default]
    LocalWrite,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ClickTargetRole {
    Link,
    Button,
    Option,
}

impl From<ClickTargetRole> for SemanticRole {
    fn from(value: ClickTargetRole) -> Self {
        match value {
            ClickTargetRole::Link => Self::Link,
            ClickTargetRole::Button => Self::Button,
            ClickTargetRole::Option => Self::Option,
        }
    }
}

impl From<ClickEffect> for SemanticEffectClass {
    fn from(value: ClickEffect) -> Self {
        match value {
            ClickEffect::Read => Self::Read,
            ClickEffect::LocalWrite => Self::LocalWrite,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClickState {
    Checked,
    Selected,
    Expanded,
    Disabled,
    Required,
    Invalid,
    Focused,
}

impl From<ClickState> for SemanticState {
    fn from(value: ClickState) -> Self {
        match value {
            ClickState::Checked => Self::Checked,
            ClickState::Selected => Self::Selected,
            ClickState::Expanded => Self::Expanded,
            ClickState::Disabled => Self::Disabled,
            ClickState::Required => Self::Required,
            ClickState::Invalid => Self::Invalid,
            ClickState::Focused => Self::Focused,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FillApproval {
    target_name: String,
    value: String,
}

struct State(Mutex<Run>);
struct Run {
    review_path: Option<std::path::PathBuf>,
    next_review_poll: Instant,
    reviewed_input: Vec<u8>,
    announced_review: Vec<AgentWorkRecord>,
    preparation: Option<mpsc::Receiver<Result<PreparedInvocation, &'static str>>>,
    invocation: Option<PreparedInvocation>,
    profile_request: Option<AgentWorkProfileRequest>,
    next_profile_poll: Instant,
    stop_observer: Option<mpsc::SyncSender<()>>,
    pinned: Option<AgentWorkProfileBinding>,
    view: Option<RetainedWorkHandle>,
    deadline: Instant,
    started: Instant,
    closed: bool,
    calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    cost_micro_usd: u64,
    #[cfg(feature = "macos-work-lifetime-diagnostic")]
    resource_failure_cause: Option<zephium_engine::WorkResourceFailureCause>,
}

enum PreparedInvocation {
    Read(PublicReadWorkInvocation),
    LocalAction(PublicLocalActionWorkInvocation),
}

/// Release-excluded actor assertion for exercising an authenticated disposable
/// profile. It is not a production identity detector; production enrollment
/// requires an independent per-document `AgentWorkAccountCollector`.
struct DevelopmentAccountSource {
    account: AgentAccountId,
}

impl AgentWorkAccountSource for DevelopmentAccountSource {
    fn sample(&self, context: ContextJoin) -> Result<AgentContextAccountBinding, AgentWorkFailure> {
        let observed_at =
            zephium_engine::work_browser_monotonic_now().ok_or(AgentWorkFailure::Contract)?;
        Ok(AgentContextAccountBinding::new(
            AgentAccountAttestationId::generate(),
            context,
            AgentAccountScope::Authenticated(self.account),
            observed_at,
        ))
    }
}

struct ApprovedFill {
    target_name: String,
    value: SemanticActionText,
}

struct ApprovedClick {
    target: ApprovedClickTarget,
    effect: SemanticEffectClass,
    verification: SemanticVerification,
}

#[derive(Eq, PartialEq)]
enum ApprovedClickTarget {
    ExactName(String),
    UniqueRoleNamePrefix { role: SemanticRole, prefix: String },
}

impl ApprovedClickTarget {
    fn matches(&self, node: &SemanticNode) -> bool {
        let Some(name) = node.name().map(SemanticText::as_str) else {
            return false;
        };
        match self {
            Self::ExactName(expected) => name == expected,
            Self::UniqueRoleNamePrefix { role, prefix } => {
                node.role() == *role && name.starts_with(prefix)
            }
        }
    }
}

struct DevelopmentLocalActionPolicy {
    origin: SemanticOrigin,
    clicks: Vec<ApprovedClick>,
    fills: Vec<ApprovedFill>,
    phase: Option<DevelopmentActionPhase>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DevelopmentActionPhase {
    OpenDialog,
    FillQuery,
    SelectResult,
    Complete,
}

impl DevelopmentLocalActionPolicy {
    fn advance_verified(
        &mut self,
        kind: SemanticActionKind,
        proof: SemanticEffectProofKind,
    ) -> Result<(), AgentWorkFailure> {
        let Some(phase) = self.phase else {
            return Ok(());
        };
        let expected = match phase {
            DevelopmentActionPhase::OpenDialog => (
                SemanticActionKind::Click,
                SemanticEffectProofKind::PageDialogOpened,
                DevelopmentActionPhase::FillQuery,
            ),
            DevelopmentActionPhase::FillQuery => (
                SemanticActionKind::Fill,
                SemanticEffectProofKind::ExactTargetValue,
                DevelopmentActionPhase::SelectResult,
            ),
            DevelopmentActionPhase::SelectResult => (
                SemanticActionKind::Click,
                SemanticEffectProofKind::PageDialogClosed,
                DevelopmentActionPhase::Complete,
            ),
            DevelopmentActionPhase::Complete => return Err(AgentWorkFailure::Contract),
        };
        if kind != expected.0 || proof != expected.1 {
            return Err(AgentWorkFailure::Contract);
        }
        self.phase = Some(expected.2);
        Ok(())
    }
}

/// Release-excluded exact intent fixture. It approves only an unambiguous,
/// public local interaction with a frozen postcondition and never grants a
/// remote write.
impl AgentWorkLocalActionPolicy for DevelopmentLocalActionPolicy {
    fn model_action_operations(
        &self,
        node: &SemanticNode,
        observation: &SemanticObservation,
    ) -> Result<SemanticOperations, AgentWorkFailure> {
        if node.sensitivity() != SemanticSensitivity::Public {
            return Ok(SemanticOperations::NONE);
        }
        let mut approved = Vec::with_capacity(2);
        if node.operations().contains(SemanticOperationClass::Click)
            && self.clicks.iter().any(|click| {
                click.verification
                    == match self.phase {
                        None => click.verification,
                        Some(DevelopmentActionPhase::OpenDialog) => {
                            SemanticVerification::PageDialogOpened
                        }
                        Some(DevelopmentActionPhase::SelectResult) => {
                            SemanticVerification::PageDialogClosed
                        }
                        _ => return false,
                    }
                    && click.target.matches(node)
                    && unique_public_operation_match(
                        observation,
                        SemanticOperationClass::Click,
                        |candidate| click.target.matches(candidate),
                    )
            })
        {
            approved.push(SemanticOperationClass::Click);
        }
        if node.operations().contains(SemanticOperationClass::Fill)
            && matches!(self.phase, None | Some(DevelopmentActionPhase::FillQuery))
            && node.name().is_some_and(|name| {
                self.fills.iter().any(|fill| {
                    fill.target_name == name.as_str()
                        && unique_public_operation_match(
                            observation,
                            SemanticOperationClass::Fill,
                            |candidate| {
                                candidate
                                    .name()
                                    .is_some_and(|candidate| candidate.as_str() == fill.target_name)
                            },
                        )
                })
            })
        {
            approved.push(SemanticOperationClass::Fill);
        }
        SemanticOperations::try_new(&approved).map_err(|_| AgentWorkFailure::Contract)
    }

    fn assess(
        &self,
        action: &SemanticPreparedAction,
        observation: &SemanticObservation,
    ) -> Result<AgentEffectAssessment, AgentWorkFailure> {
        if action.target_sensitivity() != SemanticSensitivity::Public {
            return Err(AgentWorkFailure::Contract);
        }
        let operation = match action.kind() {
            SemanticActionKind::Click => SemanticOperationClass::Click,
            SemanticActionKind::Fill => SemanticOperationClass::Fill,
            _ => return Err(AgentWorkFailure::Contract),
        };
        let frame = observation
            .reference_frame(action.target_reference())
            .map_err(|_| AgentWorkFailure::Contract)?;
        let node = observation
            .resolve(action.target_reference(), frame, operation)
            .map_err(|_| AgentWorkFailure::Contract)?;
        let (actual_effect, matching_targets) = match action.kind() {
            SemanticActionKind::Click => {
                let approved = self
                    .clicks
                    .iter()
                    .find(|approved| {
                        approved.target.matches(node)
                            && approved.verification == action.verification()
                    })
                    .ok_or(AgentWorkFailure::Contract)?;
                let matches = observation
                    .frames()
                    .iter()
                    .flat_map(|snapshot| snapshot.nodes())
                    .filter(|candidate| {
                        approved.target.matches(candidate)
                            && candidate.operations().contains(operation)
                            && candidate.sensitivity() == SemanticSensitivity::Public
                    })
                    .take(2)
                    .count();
                (approved.effect, matches)
            }
            SemanticActionKind::Fill => {
                if action.verification() != SemanticVerification::TargetValueMatchesInput {
                    return Err(AgentWorkFailure::Contract);
                }
                let Some(value) = action.fill_text() else {
                    return Err(AgentWorkFailure::Contract);
                };
                let Some(name) = node.name().map(SemanticText::as_str) else {
                    return Err(AgentWorkFailure::Contract);
                };
                let effect = self
                    .fills
                    .iter()
                    .any(|approved| approved.target_name == name && approved.value == *value)
                    .then_some(SemanticEffectClass::LocalWrite)
                    .ok_or(AgentWorkFailure::Contract)?;
                let matches = observation
                    .frames()
                    .iter()
                    .flat_map(|snapshot| snapshot.nodes())
                    .filter(|candidate| {
                        candidate
                            .name()
                            .is_some_and(|candidate| candidate.as_str() == name)
                            && candidate.operations().contains(operation)
                            && candidate.sensitivity() == SemanticSensitivity::Public
                    })
                    .take(2)
                    .count();
                (effect, matches)
            }
            _ => return Err(AgentWorkFailure::Contract),
        };
        if actual_effect != action.effect() || matching_targets != 1 {
            return Err(AgentWorkFailure::Contract);
        }
        Ok(AgentEffectAssessment::new(
            action,
            self.origin.clone(),
            actual_effect,
        ))
    }

    fn accept_verified_action(
        &mut self,
        result: &SemanticActionBatchResult,
        _: &SemanticObservation,
    ) -> Result<(), AgentWorkFailure> {
        if result.outcome() != SemanticActionBatchOutcome::Complete
            || result.total() != 1
            || result.completions().len() != 1
        {
            return Err(AgentWorkFailure::Contract);
        }
        let completion = result.completions()[0];
        self.advance_verified(completion.kind(), completion.proof())
    }

    fn terminal_extraction_ready(&self) -> bool {
        matches!(self.phase, None | Some(DevelopmentActionPhase::Complete))
    }
}

fn unique_public_operation_match(
    observation: &SemanticObservation,
    operation: SemanticOperationClass,
    matches: impl Fn(&SemanticNode) -> bool,
) -> bool {
    observation
        .frames()
        .iter()
        .flat_map(|snapshot| snapshot.nodes())
        .filter(|candidate| {
            candidate.sensitivity() == SemanticSensitivity::Public
                && candidate.operations().contains(operation)
                && matches(candidate)
        })
        .take(2)
        .count()
        == 1
}

/// No file means no runner. All request data comes from the caller's bounded
/// JSON file; the Keychain is the only credential source.
pub(super) fn install(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::var_os(CONFIG_ENV) else {
        return Ok(());
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let request = parse(&bytes).map_err(std::io::Error::other)?;
    let started = Instant::now();
    let deadline = started
        .checked_add(Duration::from_secs(request.deadline_seconds))
        .ok_or_else(|| std::io::Error::other("invalid Work deadline"))?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let (stop_observer, stop_receiver) = mpsc::sync_channel(1);
    let handle = app.clone();
    // Keychain interaction may require human input. The main/native thread
    // stays responsive; this thread can only prepare a dormant invocation.
    std::thread::Builder::new()
        .name("work-dev-credential".into())
        .spawn(move || {
            let prepared = prepare(request, deadline);
            if sender.send(prepared).is_err() {
                return;
            }
            // A bounded developer observer wake, independent of page frames.
            // It drives no tools and exits as soon as its owner drops the sender.
            let pending = Arc::new(AtomicBool::new(false));
            loop {
                // Keep at most one queued wake even if the main thread stalls.
                if !pending.swap(true, Ordering::AcqRel) {
                    let delivered = pending.clone();
                    if handle
                        .run_on_main_thread(move || delivered.store(false, Ordering::Release))
                        .is_err()
                    {
                        break;
                    }
                }
                if Instant::now() > deadline + Duration::from_secs(10) {
                    break;
                }
                if !matches!(
                    stop_receiver.recv_timeout(Duration::from_millis(100)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    break;
                }
            }
        })?;
    if !app.manage(State(Mutex::new(Run {
        review_path: std::env::var_os(REVIEW_ENV).map(Into::into),
        next_review_poll: started,
        reviewed_input: Vec::new(),
        announced_review: Vec::new(),
        preparation: Some(receiver),
        invocation: None,
        profile_request: None,
        next_profile_poll: started,
        stop_observer: Some(stop_observer),
        pinned: None,
        view: None,
        deadline,
        started,
        closed: false,
        calls: 0,
        input_tokens: 0,
        output_tokens: 0,
        cost_micro_usd: 0,
        #[cfg(feature = "macos-work-lifetime-diagnostic")]
        resource_failure_cause: None,
    }))) {
        return Err(std::io::Error::other("Work developer adapter already installed").into());
    }
    emit(json!({"work_development":"preparing", "credential":"keychain", "content":"redacted"}));
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<Request, &'static str> {
    if bytes.len() as u64 > CONFIG_BYTES {
        return Err("Work request exceeds byte limit");
    }
    let request: Request =
        serde_json::from_slice(bytes).map_err(|_| "invalid Work request JSON")?;
    if request.objective.trim().is_empty() || !(1..=600).contains(&request.deadline_seconds) {
        return Err("invalid Work objective or deadline");
    }
    Ok(request)
}

fn prepare(request: Request, deadline: Instant) -> Result<PreparedInvocation, &'static str> {
    let Request {
        objective,
        account,
        account_id,
        start_url,
        path_prefix,
        max_hops,
        output_fields,
        model,
        max_model_calls,
        operations,
        model_tokens,
        cost_micro_usd,
        deadline_seconds: _,
        inspectable_public,
        persist_result,
        local_actions,
    } = request;
    let navigation = AgentNavigationDiscovery::try_new(
        ContextNavigationTarget::parse(&start_url).map_err(|_| "invalid starting URL")?,
        path_prefix,
        max_hops,
    )
    .map_err(|_| "invalid navigation scope")?;
    let origin = navigation.origin().clone();
    let fields = output_fields
        .into_iter()
        .map(|field| {
            SemanticExtractionFieldSchema::try_text(field.name, true, field.max_bytes)
                .map_err(|_| "invalid result field")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let budget = AgentRunBudget::try_new(operations, model_tokens, cost_micro_usd, 1)
        .map_err(|_| "invalid Work budget")?;
    let account = prepare_account(account, account_id)?;
    let local_actions = local_actions
        .map(|actions| prepare_local_actions(origin, actions))
        .transpose()?;
    let credential = load_macos_probe_openai_credential()
        .map_err(|_| "development Keychain credential unavailable")?;
    if Instant::now() >= deadline {
        return Err("Work deadline elapsed during preparation");
    }
    let objective = PublicReadWorkObjective {
        objective,
        navigation,
        output_fields: fields,
    };
    let settings = PublicReadWorkSettings {
        account,
        model: match model {
            Model::Luna => AgentBrowserModel::Luna,
            Model::Terra => AgentBrowserModel::Terra,
        },
        budget,
        max_model_calls,
        deadline,
    };
    let config = AgentWorkApplicationConfig::new(
        AgentRuntimeConfig::STANDARD,
        AgentProviderTransportConfig::STANDARD,
    );
    let mut prepared = if let Some((policy, max_actions)) = local_actions {
        PreparedInvocation::LocalAction(
            PublicLocalActionWorkInvocation::try_new(
                objective,
                settings,
                config,
                credential,
                Box::new(policy),
                max_actions,
            )
            .map_err(|_| "invalid local-action Work invocation")?,
        )
    } else {
        PreparedInvocation::Read(PublicReadWorkInvocation::new(
            objective, settings, config, credential,
        ))
    };
    if persist_result {
        prepared = match prepared {
            PreparedInvocation::Read(invocation) => {
                PreparedInvocation::Read(invocation.with_persistent_result())
            }
            PreparedInvocation::LocalAction(invocation) => {
                PreparedInvocation::LocalAction(invocation.with_persistent_result())
            }
        };
    }
    if inspectable_public {
        prepared = match prepared {
            PreparedInvocation::Read(invocation) => {
                PreparedInvocation::Read(invocation.with_inspectable_public_retention())
            }
            PreparedInvocation::LocalAction(invocation) => {
                PreparedInvocation::LocalAction(invocation.with_inspectable_public_retention())
            }
        };
    }
    Ok(prepared)
}

fn prepare_account(
    account: Account,
    account_id: Option<String>,
) -> Result<PublicReadWorkAccount, &'static str> {
    Ok(match (account, account_id) {
        (Account::AnonymousPublic, None) => PublicReadWorkAccount::Anonymous,
        (Account::ActorAttestedDevelopment, Some(account)) => {
            let account =
                AgentAccountId::parse(&account).ok_or("invalid development account identity")?;
            PublicReadWorkAccount::Identified {
                account,
                source: Box::new(DevelopmentAccountSource { account }),
            }
        }
        _ => return Err("account identity does not match account mode"),
    })
}

fn prepare_local_actions(
    origin: SemanticOrigin,
    approval: LocalActionApproval,
) -> Result<(DevelopmentLocalActionPolicy, u64), &'static str> {
    if approval.clicks.len() + approval.fills.len() == 0
        || approval.clicks.len() + approval.fills.len() > 16
    {
        return Err("invalid local action approval count");
    }
    let mut clicks = Vec::with_capacity(approval.clicks.len());
    for click in approval.clicks {
        let target = match (
            click.target_name,
            click.target_name_prefix,
            click.target_role,
        ) {
            (Some(name), None, None) => {
                validate_target_name(&name)?;
                ApprovedClickTarget::ExactName(name)
            }
            (None, Some(prefix), Some(role)) => {
                validate_target_name(&prefix)?;
                ApprovedClickTarget::UniqueRoleNamePrefix {
                    role: role.into(),
                    prefix,
                }
            }
            _ => return Err("invalid local click target"),
        };
        let verification = click.verification.into();
        if clicks.iter().any(|existing: &ApprovedClick| {
            existing.target == target && existing.verification == verification
        }) {
            // One intent cannot carry conflicting classifications, even when
            // the caller supplies different effects for otherwise equal clicks.
            return Err("duplicate local click approval");
        }
        clicks.push(ApprovedClick {
            target,
            effect: click.effect.into(),
            verification,
        });
    }
    let mut fills = Vec::with_capacity(approval.fills.len());
    for fill in approval.fills {
        validate_target_name(&fill.target_name)?;
        let value =
            SemanticActionText::try_new(fill.value).map_err(|_| "invalid local fill value")?;
        if fills.iter().any(|existing: &ApprovedFill| {
            existing.target_name == fill.target_name && existing.value == value
        }) {
            return Err("duplicate local fill approval");
        }
        fills.push(ApprovedFill {
            target_name: fill.target_name,
            value,
        });
    }
    let phase = match approval.required_sequence {
        Some(RequiredLocalActionSequence::DialogSearchResult) => {
            if !clicks
                .iter()
                .any(|click| click.verification == SemanticVerification::PageDialogOpened)
                || !clicks
                    .iter()
                    .any(|click| click.verification == SemanticVerification::PageDialogClosed)
                || fills.is_empty()
            {
                return Err("dialog search sequence is incomplete");
            }
            Some(DevelopmentActionPhase::OpenDialog)
        }
        None => None,
    };
    Ok((
        DevelopmentLocalActionPolicy {
            origin,
            clicks,
            fills,
            phase,
        },
        approval.max_actions,
    ))
}

fn validate_target_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("invalid local action target name");
    }
    Ok(())
}

pub(super) fn on_run_event(app: &tauri::AppHandle, event: &tauri::RunEvent) {
    let Some(state) = app.try_state::<State>() else {
        return;
    };
    let Ok(mut run) = state.0.lock() else {
        return;
    };
    if matches!(
        event,
        tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
    ) {
        run.close();
        return;
    }
    if !run.closed
        && matches!(
            event,
            tauri::RunEvent::Ready | tauri::RunEvent::MainEventsCleared
        )
    {
        if let Err(reason) = run.poll(app) {
            emit(json!({"work_development":"stopped", "reason":reason}));
            run.close();
        }
    }
}

impl Run {
    fn close(&mut self) {
        self.closed = true;
        self.preparation.take();
        self.invocation.take();
        self.profile_request.take();
        self.stop_observer.take();
        if let Some(view) = &self.view {
            let _ = view.stop();
        }
    }

    fn poll(&mut self, app: &tauri::AppHandle) -> Result<(), &'static str> {
        if Instant::now() > self.deadline + Duration::from_secs(10) {
            return Err("observer deadline handoff to application cleanup");
        }
        if self.view.is_none() {
            if Instant::now() >= self.deadline {
                return Err("admission deadline elapsed");
            }
            if let Some(receiver) = &self.preparation {
                match receiver.try_recv() {
                    Ok(result) => {
                        self.invocation = Some(result?);
                        self.preparation = None;
                        self.next_profile_poll = Instant::now() + FOREGROUND_GRACE;
                        emit(json!({
                            "work_development":"prepared",
                            "next":"focus_zephium",
                            "grace_millis":FOREGROUND_GRACE.as_millis()
                        }));
                        return Ok(());
                    }
                    Err(mpsc::TryRecvError::Empty) => return Ok(()),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("preparation disconnected")
                    }
                }
            }
            if self.profile_request.is_none() {
                if Instant::now() < self.next_profile_poll {
                    return Ok(());
                }
                self.next_profile_poll = Instant::now() + Duration::from_millis(100);
                self.profile_request = Some(
                    super::work::selected_work_profile(app)
                        .map_err(|_| "profile query unavailable")?,
                );
            }
            let Some(readiness) = self
                .profile_request
                .as_ref()
                .and_then(|request| request.try_recv())
            else {
                return Ok(());
            };
            self.profile_request = None;
            let binding = match readiness {
                AgentWorkProfileReadiness::Ready(binding)
                | AgentWorkProfileReadiness::PolicyPending(binding) => binding,
                AgentWorkProfileReadiness::ProfileMissing
                | AgentWorkProfileReadiness::PolicyMissing
                    if self.pinned.is_none() =>
                {
                    return Ok(())
                }
                _ => return Err("selected profile unavailable"),
            };
            if self.pinned.is_some_and(|prior| prior != binding) {
                return Err("selected profile changed");
            }
            self.pinned = Some(binding);
            if !matches!(readiness, AgentWorkProfileReadiness::Ready(_)) {
                return Ok(());
            }
            let invocation = self.invocation.take().ok_or("missing invocation")?;
            self.view = Some(
                match invocation {
                    PreparedInvocation::Read(invocation) => {
                        super::work::launch_public_read_work(app, binding, invocation)
                    }
                    PreparedInvocation::LocalAction(invocation) => {
                        super::work::launch_public_local_action_work(app, binding, invocation)
                    }
                }
                .map_err(|_| "Work admission refused")?,
            );
            emit(json!({"work_development":"queued", "content":"redacted"}));
        }
        let view = self.view.as_ref().ok_or("missing Work handle")?;
        let mut drained = 0;
        for _ in 0..zephium_agent_controller::MAX_AGENT_WORK_EVENTS {
            let Some(event) = view.take_event() else {
                break;
            };
            drained += 1;
            if let AgentWorkEventKind::ModelSettled {
                input_tokens,
                output_tokens,
                cost_micro_usd,
                ..
            } = event.kind()
            {
                self.calls += 1;
                self.input_tokens += input_tokens;
                self.output_tokens += output_tokens;
                self.cost_micro_usd += cost_micro_usd;
            }
            emit(
                json!({"work_event":format!("{:?}", event.kind()), "sequence":event.sequence(), "elapsed_ms":event.elapsed_millis()}),
            );
        }
        let snapshot = view.snapshot();
        #[cfg(feature = "macos-work-lifetime-diagnostic")]
        if self.resource_failure_cause.is_none() {
            // Capture from the exact original resource while it is retained.
            // Shutdown may remove the native owner before the terminal report.
            self.resource_failure_cause = super::work::retained_resource_failure_cause(app, view);
        }
        if snapshot.phase == RetainedWorkPhase::NeedsReview {
            let records = view.records();
            if records != self.announced_review {
                for record in records
                    .iter()
                    .filter(|record| record.disposition() == AgentWorkDisposition::Interrupted)
                {
                    emit(
                        json!({"work_development":"historical_review_required", "record":record_hex(*record), "debt":record.debt().bits(), "review_file_configured":self.review_path.is_some(), "execution_started":false}),
                    );
                }
                self.announced_review = records;
            }
            if Instant::now() >= self.next_review_poll {
                self.next_review_poll = Instant::now() + Duration::from_secs(1);
                if let Some(path) = &self.review_path {
                    match std::fs::File::open(path) {
                        Ok(file) => {
                            let mut bytes = Vec::new();
                            if file.take(REVIEW_BYTES + 1).read_to_end(&mut bytes).is_ok()
                                && bytes != self.reviewed_input
                            {
                                let accepted = parse_review(&bytes)
                                    .is_ok_and(|(record, decision)| view.review(record, decision));
                                emit(
                                    json!({"work_development":"historical_review_submitted", "queued":accepted, "debt_cleared":false}),
                                );
                                self.reviewed_input = bytes;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => {
                            emit(json!({"work_development":"historical_review_file_unavailable"}))
                        }
                    }
                }
            }
        }
        if let Some(result) = view.take_extraction() {
            emit_result(&result)?;
        }
        if matches!(
            snapshot.phase,
            RetainedWorkPhase::Terminal | RetainedWorkPhase::Refused | RetainedWorkPhase::Uncertain
        ) && drained < zephium_agent_controller::MAX_AGENT_WORK_EVENTS
        {
            #[cfg(feature = "macos-work-lifetime-diagnostic")]
            emit(json!({
                "work_development":"resource_failure",
                "cause":format!("{:?}", self.resource_failure_cause),
                "content":"redacted"
            }));
            emit(
                json!({"work_development":"terminal", "phase":format!("{:?}", snapshot.phase), "failure":format!("{:?}", snapshot.failure), "record":format!("{:?}", snapshot.record), "artifact":format!("{:?}", snapshot.artifact), "usage_basis":"settled_model_events", "model_calls":self.calls, "input_tokens":self.input_tokens, "output_tokens":self.output_tokens, "cost_micro_usd":self.cost_micro_usd, "elapsed_ms":self.started.elapsed().as_millis()}),
            );
            self.closed = true;
            self.stop_observer.take();
        }
        Ok(())
    }
}

fn emit_result(result: &SemanticOwnedExtractionResult) -> Result<(), &'static str> {
    emit(
        json!({"work_result":{"trust":"ModelMapped", "requires_human_evaluation":true, "fields":result.fields().len()}}),
    );
    for field in result.fields() {
        let SemanticExtractedValue::Text(text) = field.value() else {
            return Err("unexpected result field type");
        };
        let sources = result.sources(text.source_span()).ok_or("missing source evidence")?.take(4).map(|source| json!({
            "origin":source.frame.origin().as_url().as_str(), "navigation_epoch":source.frame.context().navigation_epoch().get(), "frame_generation":source.frame.frame_generation().get(), "quote":match &source.content {
                SemanticOwnedReadContent::Text(value) => preview(value),
                SemanticOwnedReadContent::ValuePreview { text, .. } => preview(text),
                _ => "",
            }, "role":format!("{:?}", source.role), "observation":format!("{:?}", source.observation),
        })).collect::<Vec<_>>();
        emit(
            json!({"work_result_field":{"name":field.name(), "text":text.as_str(), "source_preview_limit":4,"quote_byte_limit":512,"sources":sources}}),
        );
    }
    Ok(())
}

fn preview(value: &str) -> &str {
    let mut end = value.len().min(512);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn emit(value: serde_json::Value) {
    let Ok(bytes) = serde_json::to_vec(&value) else {
        return;
    };
    let mut output = std::io::stdout().lock();
    if bytes.len() > OUTPUT_BYTES {
        let _ = output.write_all(b"{\"work_development\":\"output_exceeds_limit\"}\n");
        return;
    }
    let _ = output.write_all(&bytes);
    let _ = output.write_all(b"\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action_observation(dialog_open: bool) -> SemanticObservation {
        let identity = ContextIdentity::new(
            ContextId::from_raw(41),
            ContextRunId::from_raw(42),
            ProfileId::from(43),
            ContextKind::Owned,
        );
        let mut registry = ContextRegistry::new();
        registry
            .reserve(
                identity,
                ContextCapabilities::try_new(
                    ContextKind::Owned,
                    &[ContextCapability::Observe, ContextCapability::Act],
                )
                .unwrap(),
            )
            .unwrap();
        let operation = registry
            .begin_context(identity.id(), ContextOperationId::new(1).unwrap())
            .unwrap();
        registry
            .settle_construction(identity.id(), operation, ContextSettlement::Applied)
            .unwrap();
        let context = registry.join(identity.id()).unwrap();
        let frame = SemanticFrameJoin::try_new(
            context,
            FrameId::MAIN,
            context.frame_generation(),
            SemanticOrigin::parse("https://app.notion.com").unwrap(),
            SemanticFrameTrust::SameOrigin,
        )
        .unwrap();
        let wire: &[u8] = if dialog_open {
            br#"{"v":1,"i":1,"g":1,"c":"complete","n":[{"k":1,"r":"document","o":16},{"k":2,"p":0,"r":"textbox","s":64,"o":3,"v":{"k":"text","value":""}},{"k":3,"p":0,"r":"textbox","n":"Page editor","s":64,"o":3,"v":{"k":"text","value":""}},{"k":4,"p":0,"r":"searchbox","n":"Search","s":64,"o":3,"v":{"k":"text","value":""}}]}"#
        } else {
            br#"{"v":1,"i":1,"g":1,"c":"complete","n":[{"k":1,"r":"document","o":16},{"k":2,"p":0,"r":"button","n":"Search","o":1},{"k":3,"p":0,"r":"textbox","s":64,"o":3,"v":{"k":"text","value":""}},{"k":4,"p":0,"r":"textbox","n":"Page editor","s":64,"o":3,"v":{"k":"text","value":""}}]}"#
        };
        let snapshot = decode_semantic_snapshot(
            SemanticDecodeContext::new(
                SemanticInvocationId::new(1).unwrap(),
                frame,
                SemanticSnapshotGeneration::new(1).unwrap(),
            ),
            wire,
        )
        .unwrap();
        SemanticObservationAssembler::new(
            SemanticObservationRequest::initial(
                SemanticObservationId::new(1).unwrap(),
                context,
                SemanticObservationBudget::INITIAL_FILTERED,
            ),
            snapshot,
        )
        .unwrap()
        .finish()
        .unwrap()
    }

    fn search_result_observation(duplicate_result: bool) -> SemanticObservation {
        let identity = ContextIdentity::new(
            ContextId::from_raw(51),
            ContextRunId::from_raw(52),
            ProfileId::from(53),
            ContextKind::Owned,
        );
        let mut registry = ContextRegistry::new();
        registry
            .reserve(
                identity,
                ContextCapabilities::try_new(
                    ContextKind::Owned,
                    &[ContextCapability::Observe, ContextCapability::Act],
                )
                .unwrap(),
            )
            .unwrap();
        let operation = registry
            .begin_context(identity.id(), ContextOperationId::new(1).unwrap())
            .unwrap();
        registry
            .settle_construction(identity.id(), operation, ContextSettlement::Applied)
            .unwrap();
        let context = registry.join(identity.id()).unwrap();
        let frame = SemanticFrameJoin::try_new(
            context,
            FrameId::MAIN,
            context.frame_generation(),
            SemanticOrigin::parse("https://app.notion.com").unwrap(),
            SemanticFrameTrust::SameOrigin,
        )
        .unwrap();
        let nodes = if duplicate_result {
            r#"[{"k":1,"r":"document","o":16},{"k":2,"p":0,"r":"dialog","n":"Search Notion"},{"k":3,"p":1,"r":"combobox","n":"Search","o":2,"v":{"k":"text","value":"Quasar Agent Qualification"}},{"k":4,"p":1,"r":"option","n":"Quasar Agent Qualification Current Page 3nigma • Edited 20h ago","o":5},{"k":5,"p":1,"r":"option","n":"Quasar Agent Qualification Current Page duplicate","o":5},{"k":6,"p":0,"r":"button","n":"Quasar Agent Qualification","o":1}]"#
        } else {
            r#"[{"k":1,"r":"document","o":16},{"k":2,"p":0,"r":"dialog","n":"Search Notion"},{"k":3,"p":1,"r":"combobox","n":"Search","o":2,"v":{"k":"text","value":"Quasar Agent Qualification"}},{"k":4,"p":1,"r":"option","n":"Quasar Agent Qualification Current Page 3nigma • Edited 20h ago","o":5},{"k":5,"p":0,"r":"button","n":"Quasar Agent Qualification","o":1}]"#
        };
        let wire = format!(r#"{{"v":1,"i":1,"g":1,"c":"complete","n":{nodes}}}"#);
        let snapshot = decode_semantic_snapshot(
            SemanticDecodeContext::new(
                SemanticInvocationId::new(1).unwrap(),
                frame,
                SemanticSnapshotGeneration::new(1).unwrap(),
            ),
            wire.as_bytes(),
        )
        .unwrap();
        SemanticObservationAssembler::new(
            SemanticObservationRequest::initial(
                SemanticObservationId::new(1).unwrap(),
                context,
                SemanticObservationBudget::INITIAL_FILTERED,
            ),
            snapshot,
        )
        .unwrap()
        .finish()
        .unwrap()
    }
    #[test]
    fn dynamic_input_rejects_routes_or_answer_oracles() {
        let mut input = json!({"objective":"Find relevant guidance and explain it", "account":"anonymous_public", "start_url":"https://example.test/", "path_prefix":"/", "max_hops":2, "output_fields":[{"name":"answer","max_bytes":2048}], "model":"luna", "max_model_calls":24, "operations":64, "model_tokens":200000, "cost_micro_usd":500000, "deadline_seconds":300, "inspectable_public":true,"persist_result":true});
        assert!(parse(&serde_json::to_vec(&input).unwrap()).is_ok());
        input["expected_answer"] = json!("known answer");
        assert!(parse(&serde_json::to_vec(&input).unwrap()).is_err());
        assert!(parse(&vec![b' '; CONFIG_BYTES as usize + 1]).is_err());
    }

    #[test]
    fn development_account_assurance_is_explicit_and_cross_field_exact() {
        assert!(matches!(
            prepare_account(Account::AnonymousPublic, None).unwrap(),
            PublicReadWorkAccount::Anonymous
        ));
        assert!(prepare_account(
            Account::AnonymousPublic,
            Some("00000000000000000000000033".into())
        )
        .is_err());
        assert!(prepare_account(Account::ActorAttestedDevelopment, None).is_err());
        assert!(prepare_account(
            Account::ActorAttestedDevelopment,
            Some("not-an-account-id".into())
        )
        .is_err());
        assert!(matches!(
            prepare_account(
                Account::ActorAttestedDevelopment,
                Some("00000000000000000000000033".into())
            )
            .unwrap(),
            PublicReadWorkAccount::Identified { .. }
        ));
    }

    #[test]
    fn local_action_fixture_freezes_bounded_unique_intent() {
        let origin = SemanticOrigin::parse("https://app.notion.com").unwrap();
        let (policy, maximum) = prepare_local_actions(
            origin.clone(),
            LocalActionApproval {
                max_actions: 2,
                required_sequence: None,
                clicks: vec![ClickApproval {
                    target_name: Some("Search".into()),
                    target_name_prefix: None,
                    target_role: None,
                    effect: ClickEffect::Read,
                    verification: ClickVerification::PageDialogOpened {},
                }],
                fills: vec![FillApproval {
                    target_name: "Search".into(),
                    value: "Quasar Agent Qualification".into(),
                }],
            },
        )
        .unwrap();
        assert_eq!(maximum, 2);
        assert_eq!(policy.origin, origin);
        assert_eq!(policy.clicks.len(), 1);
        assert_eq!(policy.clicks[0].effect, SemanticEffectClass::Read);
        assert_eq!(
            policy.clicks[0].verification,
            SemanticVerification::PageDialogOpened
        );
        assert_eq!(policy.fills.len(), 1);
        assert!(prepare_local_actions(
            SemanticOrigin::parse("https://app.notion.com").unwrap(),
            LocalActionApproval {
                max_actions: 2,
                required_sequence: None,
                clicks: vec![
                    ClickApproval {
                        target_name: Some("Search".into()),
                        target_name_prefix: None,
                        target_role: None,
                        effect: ClickEffect::Read,
                        verification: ClickVerification::PageDialogOpened {},
                    },
                    ClickApproval {
                        target_name: Some("Search".into()),
                        target_name_prefix: None,
                        target_role: None,
                        effect: ClickEffect::LocalWrite,
                        verification: ClickVerification::PageDialogOpened {},
                    },
                ],
                fills: Vec::new(),
            }
        )
        .is_err());
        assert!(prepare_local_actions(
            SemanticOrigin::parse("https://app.notion.com").unwrap(),
            LocalActionApproval {
                max_actions: 2,
                required_sequence: None,
                clicks: Vec::new(),
                fills: vec![
                    FillApproval {
                        target_name: "Search".into(),
                        value: "same".into(),
                    },
                    FillApproval {
                        target_name: "Search".into(),
                        value: "same".into(),
                    },
                ],
            }
        )
        .is_err());
    }

    #[test]
    fn local_action_projection_hides_unapproved_fillable_page_editors() {
        let (policy, _) = prepare_local_actions(
            SemanticOrigin::parse("https://app.notion.com").unwrap(),
            LocalActionApproval {
                max_actions: 2,
                required_sequence: None,
                clicks: vec![ClickApproval {
                    target_name: Some("Search".into()),
                    target_name_prefix: None,
                    target_role: None,
                    effect: ClickEffect::Read,
                    verification: ClickVerification::PageDialogOpened {},
                }],
                fills: vec![FillApproval {
                    target_name: "Search".into(),
                    value: "Quasar Agent Qualification".into(),
                }],
            },
        )
        .unwrap();
        let observation = action_observation(false);
        let nodes = observation.frames()[0].nodes();
        assert_eq!(
            policy
                .model_action_operations(&nodes[1], &observation)
                .unwrap(),
            SemanticOperations::try_new(&[SemanticOperationClass::Click]).unwrap()
        );
        for node in [&nodes[2], &nodes[3]] {
            assert_eq!(
                policy.model_action_operations(node, &observation).unwrap(),
                SemanticOperations::NONE,
                "semantically fillable but unapproved page content must stay readable only"
            );
        }
        let observation = action_observation(true);
        let nodes = observation.frames()[0].nodes();
        for node in [&nodes[1], &nodes[2]] {
            assert_eq!(
                policy.model_action_operations(node, &observation).unwrap(),
                SemanticOperations::NONE,
                "semantically fillable but unapproved page content must stay readable only"
            );
        }
        assert_eq!(
            policy
                .model_action_operations(&nodes[3], &observation)
                .unwrap(),
            SemanticOperations::try_new(&[
                SemanticOperationClass::Click,
                SemanticOperationClass::Fill,
            ])
            .unwrap()
        );
    }

    #[test]
    fn result_selection_requires_one_role_scoped_prefix_match() {
        let (mut policy, maximum) = prepare_local_actions(
            SemanticOrigin::parse("https://app.notion.com").unwrap(),
            LocalActionApproval {
                max_actions: 3,
                required_sequence: Some(RequiredLocalActionSequence::DialogSearchResult),
                clicks: vec![
                    ClickApproval {
                        target_name: Some("Search or ask".into()),
                        target_name_prefix: None,
                        target_role: None,
                        effect: ClickEffect::Read,
                        verification: ClickVerification::PageDialogOpened {},
                    },
                    ClickApproval {
                        target_name: None,
                        target_name_prefix: Some("Quasar Agent Qualification Current Page ".into()),
                        target_role: Some(ClickTargetRole::Option),
                        effect: ClickEffect::Read,
                        verification: ClickVerification::PageDialogClosed {},
                    },
                ],
                fills: vec![FillApproval {
                    target_name: "Search".into(),
                    value: "Quasar Agent Qualification".into(),
                }],
            },
        )
        .unwrap();
        assert_eq!(maximum, 3);
        assert!(!policy.terminal_extraction_ready());
        policy.phase = Some(DevelopmentActionPhase::SelectResult);

        let observation = search_result_observation(false);
        let nodes = observation.frames()[0].nodes();
        assert_eq!(
            policy
                .model_action_operations(&nodes[3], &observation)
                .unwrap(),
            SemanticOperations::try_new(&[SemanticOperationClass::Click]).unwrap()
        );
        assert_eq!(
            policy
                .model_action_operations(&nodes[4], &observation)
                .unwrap(),
            SemanticOperations::NONE,
            "the underlying page title is not the role-scoped search result"
        );

        let ambiguous = search_result_observation(true);
        for node in &ambiguous.frames()[0].nodes()[3..5] {
            assert_eq!(
                policy.model_action_operations(node, &ambiguous).unwrap(),
                SemanticOperations::NONE,
                "ambiguous result prefixes must fail closed"
            );
        }
    }

    #[test]
    fn required_search_sequence_advances_only_from_exact_verified_proofs() {
        let (mut policy, _) = prepare_local_actions(
            SemanticOrigin::parse("https://app.notion.com").unwrap(),
            LocalActionApproval {
                max_actions: 3,
                required_sequence: Some(RequiredLocalActionSequence::DialogSearchResult),
                clicks: vec![
                    ClickApproval {
                        target_name: Some("Search or ask".into()),
                        target_name_prefix: None,
                        target_role: None,
                        effect: ClickEffect::Read,
                        verification: ClickVerification::PageDialogOpened {},
                    },
                    ClickApproval {
                        target_name: None,
                        target_name_prefix: Some("Quasar Agent Qualification Current Page ".into()),
                        target_role: Some(ClickTargetRole::Option),
                        effect: ClickEffect::Read,
                        verification: ClickVerification::PageDialogClosed {},
                    },
                ],
                fills: vec![FillApproval {
                    target_name: "Search".into(),
                    value: "Quasar Agent Qualification".into(),
                }],
            },
        )
        .unwrap();
        assert_eq!(
            policy.advance_verified(
                SemanticActionKind::Fill,
                SemanticEffectProofKind::ExactTargetValue,
            ),
            Err(AgentWorkFailure::Contract)
        );
        assert_eq!(policy.phase, Some(DevelopmentActionPhase::OpenDialog));
        for (kind, proof, phase) in [
            (
                SemanticActionKind::Click,
                SemanticEffectProofKind::PageDialogOpened,
                DevelopmentActionPhase::FillQuery,
            ),
            (
                SemanticActionKind::Fill,
                SemanticEffectProofKind::ExactTargetValue,
                DevelopmentActionPhase::SelectResult,
            ),
            (
                SemanticActionKind::Click,
                SemanticEffectProofKind::PageDialogClosed,
                DevelopmentActionPhase::Complete,
            ),
        ] {
            policy.advance_verified(kind, proof).unwrap();
            assert_eq!(policy.phase, Some(phase));
        }
        assert!(policy.terminal_extraction_ready());
        assert_eq!(
            policy.advance_verified(
                SemanticActionKind::Click,
                SemanticEffectProofKind::PageDialogClosed,
            ),
            Err(AgentWorkFailure::Contract)
        );
    }

    #[test]
    fn click_fixture_effect_is_explicit_and_cannot_grant_remote_writes() {
        let dialog: ClickApproval = serde_json::from_str(
            r#"{"target_name":"Search","effect":"read","verification":{"kind":"page_dialog_opened"}}"#).unwrap();
        assert_eq!(
            SemanticVerification::from(dialog.verification),
            SemanticVerification::PageDialogOpened
        );
        for verification in [
            json!({"kind":"page_dialog_opened","state":"focused","present":true}),
            json!({"kind":"dialog","state":"present"}),
        ] {
            assert!(serde_json::from_value::<ClickApproval>(
                json!({"target_name":"Search", "effect":"read", "verification":verification})
            )
            .is_err());
        }
        let legacy: ClickApproval =
            serde_json::from_str(r#"{"target_name":"Search","verification":{"kind":"target_state","state":"expanded","present":true}}"#)
                .unwrap();
        assert_eq!(
            SemanticEffectClass::from(legacy.effect),
            SemanticEffectClass::LocalWrite
        );
        for effect in [
            "external_write",
            "communication",
            "purchase",
            "destructive",
            "capability_boundary",
        ] {
            let approval = json!({
                "target_name": "Search", "effect": effect,
                "verification": {"kind":"target_state", "state": "expanded", "present": true}
            });
            assert!(serde_json::from_value::<ClickApproval>(approval).is_err());
        }
    }

    #[test]
    fn evidence_preview_preserves_utf8_and_is_bounded() {
        let text = "界".repeat(200);
        assert_eq!(preview(&text).len(), 510);
        assert_eq!(preview("short evidence"), "short evidence");
    }

    #[test]
    fn recovery_review_requires_exact_interrupted_record_and_explicit_decision() {
        let mut bytes = [0; AGENT_WORK_RECORD_BYTES];
        bytes[0] = 1;
        bytes[1] = AgentWorkDisposition::Interrupted as u8;
        bytes[2] = AgentWorkDebt::UNKNOWN.bits();
        bytes[15] = 1;
        bytes[16] = 1;
        let record = AgentWorkRecord::decode(bytes).unwrap();
        let encoded = record_hex(record);
        for decision in ["accept_fresh_admission", "reject"] {
            let request =
                serde_json::to_vec(&json!({"record":encoded, "decision":decision})).unwrap();
            assert_eq!(parse_review(&request).unwrap().0, record);
        }
        for invalid in [
            json!({"record":encoded}),
            json!({"record":encoded,"decision":"resume"}),
            json!({"record":encoded,"decision":"reject","clear_debt":true}),
            json!({"record":"ff","decision":"reject"}),
            json!({"record":"界".repeat(64),"decision":"reject"}),
        ] {
            assert!(parse_review(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        bytes[1] = AgentWorkDisposition::RecoveryRequired as u8;
        let live = AgentWorkRecord::decode(bytes).unwrap();
        assert!(parse_review(
            &serde_json::to_vec(&json!({"record":record_hex(live),"decision":"reject"})).unwrap()
        )
        .is_err());
        assert!(parse_review(&vec![b' '; REVIEW_BYTES as usize + 1]).is_err());
    }
}
