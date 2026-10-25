//! Controlled open-objective proof of production native Back through the
//! shipping retained Work entry. This module is release-excluded.

use crate::native_work_clock::{authority_window, NativeWorkClock};
pub use crate::navigation_qualification::ApplicationReport;
use std::{
    io::Write as _,
    sync::Arc,
    time::{Duration, Instant},
};
use zephium_agent_controller::*;
use zephium_agent_provider_transport::{
    load_probe_openai_credential, AgentProviderTransportConfig,
};
use zephium_agent_runtime::AgentRuntimeConfig;
use zephium_agentic::*;
use zephium_app::{RetainedWorkHandle, RetainedWorkPhase};

const TOTAL: Duration = Duration::from_secs(150);
const RELEASE_CODE: &str = "ZEPH-R7-492";
const ORIGIN_HEADING: &str = "Quasar Release Card";
const REGISTER_HEADING: &str = "Compatibility Register";
pub const OBJECTIVE: &str = "Verify the release code on the current Quasar release card against the relevant linked compatibility record. After inspecting the relevant record, return to the original release card and provide the exact release code shown there. Choose the relevant source and browser operations yourself. Do not report from memory: finish by extracting schema 1 on the restored release card, with release_code copied exactly and cited only from its current heading.";

pub fn configuration_diagnostic() -> String {
    "work-retained-back-config: entry=admit_retained_trusted_work provider=OpenAIResponses model=gpt-5.6-luna reasoning=medium store=true task=retained-back-open-v1 navigation=production_observed_link history=exact_run_enrolled".into()
}

#[derive(Clone)]
struct Checkpoint {
    context: ContextJoin,
    observation: SemanticObservationId,
    generation: SemanticObservationGeneration,
    frame: SemanticFrameJoin,
    invocation: SemanticInvocationId,
    snapshot: SemanticSnapshotGeneration,
    reference: SemanticReferenceId,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    AwaitRegister,
    AwaitReturn,
    Restored,
    Complete,
}

struct BackTask {
    inner: AgentWorkDiscoveryTask,
    fixture: Option<FixtureServer>,
    origin: SemanticOrigin,
    phase: Phase,
    prior: Option<Checkpoint>,
    current: Option<Checkpoint>,
}

impl BackTask {
    fn new(
        identity: ContextIdentity,
        origin: SemanticOrigin,
        departure: ContextNavigationTarget,
        fixture: FixtureServer,
    ) -> Result<Self, AgentWorkFailure> {
        let rule = AgentNavigationOriginRule::try_new(
            origin.clone(),
            "/retained-back/".into(),
            false,
            false,
        )
        .map_err(|_| AgentWorkFailure::Contract)?;
        let discovery = AgentNavigationDiscovery::try_new_production(departure, vec![rule], 2, 2)
            .map_err(|_| AgentWorkFailure::Contract)?;
        let inner = AgentWorkDiscoveryTask::try_new(
            identity,
            discovery,
            vec![
                SemanticExtractionFieldSchema::try_text("release_code".into(), true, 32)
                    .map_err(|_| AgentWorkFailure::Contract)?,
            ],
        )?;
        Ok(Self {
            inner,
            fixture: Some(fixture),
            origin,
            phase: Phase::AwaitRegister,
            prior: None,
            current: None,
        })
    }
}

fn successor(prior: ContextJoin, next: ContextJoin) -> bool {
    prior.identity() == next.identity()
        && prior.context_generation() == next.context_generation()
        && prior.cancellation_generation() == next.cancellation_generation()
        && prior.frame() == FrameId::MAIN
        && next.frame() == FrameId::MAIN
        && prior.navigation_epoch().get().checked_add(1) == Some(next.navigation_epoch().get())
        && prior.frame_generation().get().checked_add(1) == Some(next.frame_generation().get())
}

fn exactly_one_heading<'a>(frame: &'a SemanticSnapshot, name: &str) -> Option<&'a SemanticNode> {
    let mut matches = frame.nodes().iter().filter(|node| {
        node.role() == SemanticRole::Heading
            && node.sensitivity() == SemanticSensitivity::Public
            && node.name().is_some_and(|value| value.as_str() == name)
    });
    let result = matches.next()?;
    matches.next().is_none().then_some(result)
}

impl AgentWorkTask for BackTask {
    fn navigation_discovery(&self) -> Option<&AgentNavigationDiscovery> {
        self.inner.navigation_discovery()
    }
    fn allows_baseline_read(&self) -> bool {
        self.inner.allows_baseline_read()
    }
    fn allows_progressive_observation(&self) -> bool {
        self.inner.allows_progressive_observation()
    }
    fn allows_standalone_wait(&self) -> bool {
        self.inner.allows_standalone_wait()
    }
    fn allows_viewport_screenshot(&self) -> bool {
        self.inner.allows_viewport_screenshot()
    }
    fn allows_human_request(&self) -> bool {
        self.inner.allows_human_request()
    }
    fn allows_history_back(&self) -> bool {
        self.inner.allows_history_back()
    }
    fn extraction_schema(&self) -> Option<&SemanticExtractionSchema> {
        self.inner.extraction_schema()
    }
    fn assess(
        &self,
        action: &SemanticPreparedAction,
    ) -> Result<AgentEffectAssessment, AgentWorkFailure> {
        self.inner.assess(action)
    }
    fn attest_account(
        &self,
        context: ContextJoin,
        now: AgentPolicyInstant,
    ) -> Result<AgentContextAccountBinding, AgentWorkFailure> {
        self.inner.attest_account(context, now)
    }
    fn evaluate(
        &mut self,
        observation: &SemanticObservation,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        if !self.fixture.as_ref().is_some_and(FixtureServer::is_healthy)
            || self.phase == Phase::Complete
        {
            return Err(AgentWorkFailure::Contract);
        }
        let [frame] = observation.frames() else {
            return Err(AgentWorkFailure::Contract);
        };
        if !observation.frame_boundaries().is_empty()
            || frame.completeness() != SemanticCompleteness::Complete
            || frame.frame().origin() != &self.origin
            || frame.frame().frame() != FrameId::MAIN
        {
            return Err(AgentWorkFailure::Contract);
        }
        let is_origin = exactly_one_heading(frame, ORIGIN_HEADING).is_some();
        let is_register = exactly_one_heading(frame, REGISTER_HEADING).is_some();
        if is_origin == is_register {
            return Err(AgentWorkFailure::Contract);
        }
        let release = exactly_one_heading(frame, RELEASE_CODE).ok_or(AgentWorkFailure::Contract)?;
        if is_origin {
            for link in ["Compatibility register", "Release handbook"] {
                let count = frame
                    .nodes()
                    .iter()
                    .filter(|node| {
                        node.role() == SemanticRole::Link
                            && node.name().is_some_and(|name| name.as_str() == link)
                    })
                    .count();
                if count != 1 {
                    return Err(AgentWorkFailure::Contract);
                }
            }
        }
        let checkpoint = Checkpoint {
            context: observation.request().context(),
            observation: observation.request().id(),
            generation: observation.request().generation(),
            frame: frame.frame().clone(),
            invocation: frame.invocation(),
            snapshot: frame.generation(),
            reference: release.reference(),
        };
        self.inner.evaluate(observation)?;
        let (label, back_expected) = match self.phase {
            Phase::AwaitRegister if is_origin => {
                if self
                    .prior
                    .as_ref()
                    .is_some_and(|prior| prior.context != checkpoint.context)
                {
                    return Err(AgentWorkFailure::Contract);
                }
                ("origin", false)
            }
            Phase::AwaitRegister if is_register => {
                let prior = self.prior.as_ref().ok_or(AgentWorkFailure::Contract)?;
                if !successor(prior.context, checkpoint.context)
                    || prior.observation == checkpoint.observation
                    || prior.invocation == checkpoint.invocation
                    || prior.snapshot == checkpoint.snapshot
                {
                    return Err(AgentWorkFailure::Contract);
                }
                self.phase = Phase::AwaitReturn;
                ("register", true)
            }
            Phase::AwaitReturn if is_register => {
                if self
                    .prior
                    .as_ref()
                    .is_none_or(|prior| prior.context != checkpoint.context)
                {
                    return Err(AgentWorkFailure::Contract);
                }
                ("register", true)
            }
            Phase::AwaitReturn if is_origin => {
                let prior = self.prior.as_ref().ok_or(AgentWorkFailure::Contract)?;
                if !successor(prior.context, checkpoint.context)
                    || prior.observation == checkpoint.observation
                    || prior.invocation == checkpoint.invocation
                    || prior.snapshot == checkpoint.snapshot
                {
                    return Err(AgentWorkFailure::Contract);
                }
                self.phase = Phase::Restored;
                ("restored", false)
            }
            Phase::Restored if is_origin => {
                if self
                    .prior
                    .as_ref()
                    .is_none_or(|prior| prior.context != checkpoint.context)
                {
                    return Err(AgentWorkFailure::Contract);
                }
                ("restored", false)
            }
            Phase::Complete => return Err(AgentWorkFailure::Contract),
            _ => return Err(AgentWorkFailure::Contract),
        };
        self.prior = Some(checkpoint.clone());
        self.current = Some(checkpoint.clone());
        writeln!(std::io::stdout().lock(), "work-retained-back-observation: phase={label} navigation_epoch={} frame_generation={} invocation={} back_expected={back_expected} content=redacted", checkpoint.context.navigation_epoch().get(), checkpoint.context.frame_generation().get(), checkpoint.invocation.get())
            .map_err(|_| AgentWorkFailure::Contract)?;
        Ok(AgentWorkTaskProgress::Continue)
    }
    fn accept_extraction(
        &mut self,
        result: &SemanticExtractionResult<'_>,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        let checkpoint = self.current.as_ref().ok_or(AgentWorkFailure::Contract)?;
        if self.phase != Phase::Restored
            || result.observation() != checkpoint.observation
            || result.schema().get() != 1
        {
            return Err(AgentWorkFailure::Contract);
        }
        let [field] = result.fields() else {
            return Err(AgentWorkFailure::Contract);
        };
        let SemanticExtractedValue::Text(value) = field.value() else {
            return Err(AgentWorkFailure::Contract);
        };
        let Some([source]) = result.sources(value.source_span()) else {
            return Err(AgentWorkFailure::Contract);
        };
        let fragment = source.fragment();
        let provenance = fragment.provenance();
        if field.name() != "release_code"
            || value.as_str() != RELEASE_CODE
            || provenance.observation() != checkpoint.observation
            || provenance.observation_generation() != checkpoint.generation
            || provenance.frame() != &checkpoint.frame
            || provenance.invocation() != checkpoint.invocation
            || provenance.snapshot() != checkpoint.snapshot
            || provenance.reference() != checkpoint.reference
            || provenance.origin() != &self.origin
            || fragment.role() != SemanticRole::Heading
            || !matches!(fragment.content(), SemanticReadContent::Text(text) if text.as_str() == RELEASE_CODE)
        {
            return Err(AgentWorkFailure::Contract);
        }
        let progress = self.inner.accept_extraction(result)?;
        if progress != AgentWorkTaskProgress::Complete {
            return Err(AgentWorkFailure::Contract);
        }
        self.fixture
            .take()
            .ok_or(AgentWorkFailure::Contract)?
            .shutdown()
            .map_err(|_| AgentWorkFailure::Contract)?;
        self.phase = Phase::Complete;
        Ok(progress)
    }
}

pub fn load_request(
    started: Instant,
    profile: zephium_app::AgentWorkProfileBinding,
) -> Result<crate::TrustedWorkRequest, &'static str> {
    let deadline = started.checked_add(TOTAL).ok_or("deadline")?;
    if Instant::now() >= deadline {
        return Err("deadline");
    }
    let fixture = FixtureServer::start().map_err(|_| "fixture")?;
    let departure = ContextNavigationTarget::parse(&fixture.url(FixtureRoute::RetainedBackOrigin))
        .map_err(|_| "target")?;
    let origin = SemanticOrigin::parse(departure.as_url().as_str()).map_err(|_| "origin")?;
    let identity = ContextIdentity::new(
        ContextId::generate(),
        ContextRunId::generate(),
        profile.profile(),
        ContextKind::Owned,
    );
    let input = input(
        identity,
        profile.storage_class(),
        deadline,
        departure.clone(),
        origin.clone(),
    )?;
    let task = BackTask::new(identity, origin, departure, fixture).map_err(|_| "task")?;
    let credential = load_probe_openai_credential().map_err(|_| "credential")?;
    if Instant::now() >= deadline {
        return Err("deadline");
    }
    Ok(crate::TrustedWorkRequest::new(
        input,
        zephium_app::AgentWorkApplicationConfig::new(
            AgentRuntimeConfig::STANDARD,
            AgentProviderTransportConfig::STANDARD,
        ),
        credential,
        Box::new(task),
    )
    .with_browser_profile(profile)
    .with_public_qualification_retention())
}

fn input(
    identity: ContextIdentity,
    storage: ContextProfileStorageClass,
    deadline: Instant,
    departure: ContextNavigationTarget,
    origin: SemanticOrigin,
) -> Result<AgentWorkRunInput, &'static str> {
    let effects = AgentEffectScope::try_new(&[SemanticEffectClass::Read]).map_err(|_| "effects")?;
    // Eight provider calls plus the two independently accounted native
    // navigation effects. Keep the qualification's safety ceiling aligned
    // with every route the advertised model-call budget can validly finish.
    let budget = AgentRunBudget::try_new(10, 100_000, 100_000, 1).map_err(|_| "budget")?;
    let discovery = AgentNavigationDiscovery::try_new_production(
        departure.clone(),
        vec![AgentNavigationOriginRule::try_new(
            origin.clone(),
            "/retained-back/".into(),
            false,
            false,
        )
        .map_err(|_| "discovery")?],
        2,
        2,
    )
    .map_err(|_| "discovery")?;
    let node = AgentPlanNodeId::generate();
    let authority = AgentPlanNodeAuthority::try_new(
        vec![identity.profile()],
        vec![AgentAccountScope::Anonymous],
        vec![origin.clone()],
        SemanticSensitivity::Public,
        effects,
    )
    .and_then(|authority| authority.with_navigation_discovery(discovery))
    .map_err(|_| "authority")?;
    let (issued, expires) = authority_window(deadline)?;
    let manifest = AgentRunManifest::try_new(
        AgentRunManifestId::generate(),
        identity.owner(),
        AgentRunScope::try_new(
            vec![identity.profile()],
            vec![AgentAccountScope::Anonymous],
            vec![origin],
            SemanticSensitivity::Public,
            effects,
            Vec::new(),
        )
        .map_err(|_| "scope")?,
        budget,
        issued,
        expires,
        vec![AgentPlanNodeScope::new(node, authority, budget, expires)],
    )
    .map_err(|_| "manifest")?;
    let ids = TerraControllerIds::try_new(
        AgentSupervisorId::new(1).ok_or("id")?,
        AgentSupervisorAttemptId::new(1).ok_or("id")?,
        AgentSupervisorCancellationId::new(1).ok_or("id")?,
        AgentModelCallId::new(1).ok_or("id")?,
        [1, 2, 3, 4].map(|id| AgentAuditEventId::new(id).expect("fixed nonzero ID")),
        AgentAuditDeliveryId::new(1).ok_or("id")?,
    )
    .map_err(|_| "ids")?;
    AgentWorkRunInput::try_new(
        manifest,
        AgentPlanLeaseBinding::new(AgentPlanLeaseId::generate(), node),
        AgentWorkContextSpec::try_new_with_document_policy(
            identity,
            storage,
            departure,
            WorkBrowserDocumentPolicy::Exact,
        )
        .map_err(|_| "context")?,
        OBJECTIVE.into(),
        AgentWorkRunSettings::new(
            AgentBrowserModel::Luna,
            ids,
            Arc::new(NativeWorkClock),
            deadline,
        ),
    )
    .map_err(|_| "input")
}

pub fn cancel(view: &RetainedWorkHandle) -> bool {
    view.stop()
}

#[derive(Default)]
pub struct ApplicationObserver {
    report: ApplicationReport,
    sequence: u64,
    failed: bool,
    navigate: u8,
    back: u8,
    extract: u8,
}

impl ApplicationObserver {
    pub fn report(&self) -> ApplicationReport {
        self.report
    }
    pub fn healthy(&self) -> bool {
        !self.failed
    }

    fn observe_kind(&mut self, kind: AgentWorkEventKind) {
        match kind {
            AgentWorkEventKind::ModelSettled {
                input_tokens,
                output_tokens,
                cost_micro_usd,
                ..
            } => {
                for (total, delta) in [
                    (&mut self.report.model_calls, 1),
                    (&mut self.report.input_tokens, input_tokens),
                    (&mut self.report.output_tokens, output_tokens),
                    (&mut self.report.cost_micro_usd, cost_micro_usd),
                ] {
                    if let Some(sum) = total.checked_add(delta) {
                        *total = sum;
                    } else {
                        self.failed = true;
                    }
                }
            }
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Navigate) => {
                self.navigate = self.navigate.saturating_add(1);
                self.failed |= self.navigate != 1 || self.back != 0 || self.extract != 0;
            }
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Back) => {
                self.back = self.back.saturating_add(1);
                self.failed |= self.navigate != 1 || self.back != 1 || self.extract != 0;
            }
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Extract) => {
                self.extract = self.extract.saturating_add(1);
                self.failed |= self.navigate != 1 || self.back != 1 || self.extract != 1;
            }
            AgentWorkEventKind::ToolProposed(
                AgentBrowserToolKind::Read
                | AgentBrowserToolKind::Locate
                | AgentBrowserToolKind::Snapshot,
            ) => {}
            AgentWorkEventKind::ToolProposed(_)
            | AgentWorkEventKind::ActionActive
            | AgentWorkEventKind::Verified
            | AgentWorkEventKind::NeedsHuman(_)
            | AgentWorkEventKind::Recovery => self.failed = true,
            _ => {}
        }
    }

    pub fn poll(
        &mut self,
        view: &RetainedWorkHandle,
        resource_failure: impl FnOnce() -> Option<zephium_engine::WorkResourceFailureCause>,
    ) -> Option<ApplicationReport> {
        let snapshot = view.snapshot();
        for _ in 0..256 {
            let Some(event) = view.take_event() else {
                break;
            };
            self.failed |= event.run() != snapshot.run
                || self.sequence.checked_add(1) != Some(event.sequence());
            self.sequence = event.sequence();
            self.observe_kind(event.kind());
            self.failed |= writeln!(
                std::io::stdout().lock(),
                "work-retained-back-event: sequence={} phase={:?} wall_ms={} content=redacted",
                event.sequence(),
                event.kind(),
                event.elapsed_millis()
            )
            .is_err();
        }
        if !matches!(
            snapshot.phase,
            RetainedWorkPhase::Terminal | RetainedWorkPhase::Uncertain | RetainedWorkPhase::Refused
        ) {
            return None;
        }
        let resource_failure = resource_failure();
        self.report.navigation_proposals = u64::from(self.navigate) + u64::from(self.back);
        self.report.durable_terminal_verified = snapshot.record.is_some_and(|record| {
            record.disposition() == AgentWorkDisposition::Succeeded
                && record.debt() == AgentWorkDebt::NONE
                && record.key()[16..] == snapshot.run.bytes()
        });
        self.report.source_mapping_verified = view
            .take_extraction()
            .is_some_and(|result| verify_owned(&result) && view.take_extraction().is_none());
        self.report.accepted = !self.failed
            && resource_failure.is_none()
            && snapshot.phase == RetainedWorkPhase::Terminal
            && snapshot.failure.is_none()
            && snapshot.persistence_failure.is_none()
            && self.navigate == 1
            && self.back == 1
            && self.extract == 1
            && self.report.durable_terminal_verified
            && self.report.source_mapping_verified
            && (1..=8).contains(&self.report.model_calls)
            && self
                .report
                .input_tokens
                .saturating_add(self.report.output_tokens)
                <= 100_000
            && self.report.cost_micro_usd <= 100_000;
        self.report.accepted &= writeln!(std::io::stdout().lock(), "work-retained-back-terminal: phase={:?} failure={:?} persistence_failure={:?} resource_failure={resource_failure:?} navigate={} back={} extract={} model_calls={} input_tokens={} output_tokens={} cost_micro_usd={} source_mapping={} durable={} accepted={} content=redacted", snapshot.phase, snapshot.failure, snapshot.persistence_failure, self.navigate, self.back, self.extract, self.report.model_calls, self.report.input_tokens, self.report.output_tokens, self.report.cost_micro_usd, self.report.source_mapping_verified, self.report.durable_terminal_verified, self.report.accepted).is_ok();
        Some(self.report)
    }
}

fn verify_owned(result: &SemanticOwnedExtractionResult) -> bool {
    let [field] = result.fields() else {
        return false;
    };
    let SemanticExtractedValue::Text(value) = field.value() else {
        return false;
    };
    let Some(mut sources) = result.sources(value.source_span()) else {
        return false;
    };
    let Some(source) = sources.next() else {
        return false;
    };
    result.trust() == SemanticExtractionTrust::ModelMapped
        && result.schema().get() == 1
        && field.name() == "release_code"
        && value.as_str() == RELEASE_CODE
        && sources.next().is_none()
        && source.role == SemanticRole::Heading
        && source.sensitivity == SemanticSensitivity::Public
        && source.frame.frame() == FrameId::MAIN
        && source.frame.origin().as_url().scheme() == "http"
        && source.frame.origin().as_url().host_str() == Some("127.0.0.1")
        && matches!(&source.content, SemanticOwnedReadContent::Text(text) if text == RELEASE_CODE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_back_task_freezes_required_capabilities() {
        let server = FixtureServer::start().unwrap();
        let departure =
            ContextNavigationTarget::parse(&server.url(FixtureRoute::RetainedBackOrigin)).unwrap();
        let origin = SemanticOrigin::parse(departure.as_url().as_str()).unwrap();
        let identity = ContextIdentity::new(
            ContextId::generate(),
            ContextRunId::generate(),
            1_u128.into(),
            ContextKind::Owned,
        );
        let task = BackTask::new(identity, origin, departure, server).unwrap();
        assert!(task
            .navigation_discovery()
            .is_some_and(AgentNavigationDiscovery::is_production));
        assert!(task.allows_history_back());
        assert!(task.allows_baseline_read());
        assert!(task.allows_progressive_observation());
        assert!(!task.allows_actions_before_extraction());
        assert_eq!(
            task.extraction_schema().unwrap().fields()[0].name(),
            "release_code"
        );
    }
}
