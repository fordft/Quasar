//! Explicit public-data qualification through the shipping actor and real port.

use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};
use zephium_agent_controller::*;
use zephium_agent_provider_transport::{
    load_macos_probe_openai_credential, AgentProviderTransport, AgentProviderTransportConfig,
};
use zephium_agent_runtime::{AgentRuntimeConfig, PendingAgentRuntime};
use zephium_agentic::*;

struct Clock(Instant);
impl TerraControllerClock for Clock {
    fn now(&self) -> Result<AgentPolicyInstant, TerraControllerClockError> {
        let elapsed = u64::try_from(self.0.elapsed().as_millis())
            .map_err(|_| TerraControllerClockError::Invalid)?;
        Ok(AgentPolicyInstant::from_millis(1_000 + elapsed))
    }
}

type Task = AgentWorkFormTask;
struct PublicPreparedResultTask {
    actions: Task,
    extraction: AgentWorkExtractionTask,
    ready: Option<SemanticObservationId>,
    subtree: bool,
}
impl AgentWorkTask for PublicPreparedResultTask {
    fn allows_subtree_extraction(&self) -> bool {
        self.subtree
    }
    fn allows_actions_before_extraction(&self) -> bool {
        true
    }
    fn extraction_schema(&self) -> Option<&SemanticExtractionSchema> {
        self.extraction.extraction_schema()
    }
    fn model_action_operations(
        &self,
        node: &SemanticNode,
        observation: &SemanticObservation,
    ) -> Result<SemanticOperations, AgentWorkFailure> {
        if self.ready.is_some() {
            Ok(SemanticOperations::NONE)
        } else {
            self.actions.model_action_operations(node, observation)
        }
    }
    fn evaluate(
        &mut self,
        observation: &SemanticObservation,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        if self.actions.evaluate(observation)? == AgentWorkTaskProgress::Complete {
            self.ready = Some(observation.request().id());
            Ok(AgentWorkTaskProgress::ReadyForExtraction)
        } else {
            Ok(AgentWorkTaskProgress::Continue)
        }
    }
    fn assess(
        &self,
        action: &SemanticPreparedAction,
    ) -> Result<AgentEffectAssessment, AgentWorkFailure> {
        if self.ready.is_some() {
            return Err(AgentWorkFailure::Contract);
        }
        self.actions.assess(action)
    }
    fn attest_account(
        &self,
        context: ContextJoin,
        now: AgentPolicyInstant,
    ) -> Result<AgentContextAccountBinding, AgentWorkFailure> {
        self.actions.attest_account(context, now)
    }
    fn accept_extraction(
        &mut self,
        result: &SemanticExtractionResult<'_>,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        let wrong_observation = if self.subtree {
            self.ready.is_none() || Some(result.observation()) == self.ready
        } else {
            Some(result.observation()) != self.ready
        };
        if wrong_observation {
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
        let SemanticReadContent::ValuePreview(preview) = fragment.content() else {
            return Err(AgentWorkFailure::Contract);
        };
        if value.as_str() != "Quasar open source browser"
            || !matches!(
                fragment.role(),
                SemanticRole::Searchbox | SemanticRole::Textbox
            )
            || preview.truncated()
            || preview.source_bytes() != value.as_str().len()
            || preview.text() != value.as_str()
        {
            return Err(AgentWorkFailure::Contract);
        }
        self.extraction.accept_extraction(result)
    }
}

enum PublicWorkInput {
    Site(super::work_sites::Site),
    ReadActions,
    Actions,
    ReviewActions,
    Extraction,
    Artifact,
    ActionsAndExtraction,
    ActionsAndScopedExtraction,
}
// The public test's completion contract runs before successful actor closure;
// application delivery checks the owned result again independently afterward.
struct PublicExtractionTask(AgentWorkExtractionTask);
impl AgentWorkTask for PublicExtractionTask {
    fn evaluate(
        &mut self,
        observation: &SemanticObservation,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        self.0.evaluate(observation)
    }
    fn assess(
        &self,
        action: &SemanticPreparedAction,
    ) -> Result<AgentEffectAssessment, AgentWorkFailure> {
        self.0.assess(action)
    }
    fn attest_account(
        &self,
        context: ContextJoin,
        now: AgentPolicyInstant,
    ) -> Result<AgentContextAccountBinding, AgentWorkFailure> {
        self.0.attest_account(context, now)
    }
    fn extraction_schema(&self) -> Option<&SemanticExtractionSchema> {
        self.0.extraction_schema()
    }
    fn accept_extraction(
        &mut self,
        result: &SemanticExtractionResult<'_>,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        let mut ids = std::collections::BTreeSet::new();
        let (mut english, mut german) = (false, false);
        let [field] = result.fields() else {
            return Err(AgentWorkFailure::Contract);
        };
        let SemanticExtractedValue::TextList(list) = field.value() else {
            return Err(AgentWorkFailure::Contract);
        };
        if list.items().len() != 10 {
            return Err(AgentWorkFailure::Contract);
        }
        for item in list.items() {
            let Some([source]) = result.sources(item.source_span()) else {
                return Err(AgentWorkFailure::Contract);
            };
            let fragment = source.fragment();
            let SemanticReadContent::Text(text) = fragment.content() else {
                return Err(AgentWorkFailure::Contract);
            };
            if fragment.role() != SemanticRole::Link
                || !ids.insert(fragment.id())
                || text.as_str() != item.as_str()
            {
                return Err(AgentWorkFailure::Contract);
            }
            english |= text.as_str().contains("English");
            german |= text.as_str().contains("Deutsch");
        }
        if !english || !german {
            return Err(AgentWorkFailure::Contract);
        }
        self.0.accept_extraction(result)
    }
}
pub(super) fn input(
    started: Instant,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::Actions)
}

pub(super) fn review_input(
    started: Instant,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::ReviewActions)
}

pub(super) fn read_input(
    started: Instant,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::ReadActions)
}

pub(super) fn extraction_input(
    started: Instant,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::Extraction)
}

pub(super) fn site_input(
    started: Instant,
    site: super::work_sites::Site,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::Site(site))
}

pub(super) fn artifact_input(
    started: Instant,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::Artifact)
}

pub(super) fn combined_input(
    started: Instant,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::ActionsAndExtraction)
}

pub(super) fn scoped_input(
    started: Instant,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    input_mode(started, PublicWorkInput::ActionsAndScopedExtraction)
}

fn input_mode(
    started: Instant,
    mode: PublicWorkInput,
) -> Result<
    (
        zephium_core::ids::ProfileId,
        AgentWorkRunInput,
        Box<dyn AgentWorkTask>,
    ),
    super::ProbeFailure,
> {
    use super::ProbeFailure as Error;
    let site = match mode {
        PublicWorkInput::Site(site) => Some(site),
        _ => None,
    };
    let extraction = matches!(
        mode,
        PublicWorkInput::Extraction | PublicWorkInput::Artifact | PublicWorkInput::Site(_)
    );
    let durable = matches!(mode, PublicWorkInput::Artifact);
    let subtree = matches!(mode, PublicWorkInput::ActionsAndScopedExtraction);
    let combined = matches!(
        mode,
        PublicWorkInput::ActionsAndExtraction | PublicWorkInput::ActionsAndScopedExtraction
    );
    let profile = if durable {
        zephium_core::ids::ProfileId::generate()
    } else {
        1_u128.into()
    };
    let context = ContextIdentity::new(
        ContextId::generate(),
        ContextRunId::generate(),
        profile,
        ContextKind::Owned,
    );
    let target = site.map_or("https://www.wikipedia.org/", super::work_sites::Site::url);
    let origin = SemanticOrigin::parse(target).map_err(|_| Error::Authority)?;
    let form_task = || {
        AgentWorkFormTask::try_new_local_preparation(
            context,
            origin.clone(),
            AgentAccountScope::Anonymous,
            vec![
                AgentWorkFormPhase::try_new(vec![
                    AgentWorkFormGoal::fill(None, "Quasar browser".into())?,
                    AgentWorkFormGoal::select(None, "Deutsch".into())?,
                ])?,
                AgentWorkFormPhase::try_new(vec![
                    AgentWorkFormGoal::fill(None, "Quasar open source browser".into())?,
                    AgentWorkFormGoal::select(None, "Deutsch".into())?,
                ])?,
            ],
        )
    };
    let form_task = form_task().map_err(|_| Error::Authority)?;
    let inspecting = matches!(mode, PublicWorkInput::ReadActions);
    let form_task = if inspecting {
        form_task.with_baseline_read()
    } else {
        form_task
    };
    let effects = AgentEffectScope::try_new(
        if extraction || matches!(mode, PublicWorkInput::ReviewActions) {
            &[SemanticEffectClass::Read]
        } else {
            &[SemanticEffectClass::Read, SemanticEffectClass::LocalWrite]
        },
    )
    .map_err(|_| Error::Authority)?;
    let budget =
        AgentRunBudget::try_new(24, 1_000_000, 1_000_000, 1).map_err(|_| Error::Authority)?;
    let node = AgentPlanNodeId::generate();
    let authority = AgentPlanNodeAuthority::try_new(
        vec![profile],
        vec![AgentAccountScope::Anonymous],
        vec![origin.clone()],
        SemanticSensitivity::Public,
        effects,
    )
    .map_err(|_| Error::Authority)?;
    let route = match site {
        Some(super::work_sites::Site::ReactRoute) => Some(super::work_route::route()),
        Some(super::work_sites::Site::CommerceProduct) => Some(super::work_commerce::route()),
        _ => None,
    };
    let authority = if let Some(route) = route {
        authority
            .with_navigation_route(route.map_err(|_| Error::Authority)?)
            .map_err(|_| Error::Authority)?
    } else {
        authority
    };
    let manifest = AgentRunManifest::try_new(
        AgentRunManifestId::generate(),
        context.owner(),
        AgentRunScope::try_new(
            vec![profile],
            vec![AgentAccountScope::Anonymous],
            vec![origin.clone()],
            SemanticSensitivity::Public,
            effects,
            Vec::new(),
        )
        .map_err(|_| Error::Authority)?,
        budget,
        AgentPolicyInstant::from_millis(1_000),
        AgentPolicyInstant::from_millis(200_000),
        vec![AgentPlanNodeScope::new(
            node,
            authority,
            budget,
            AgentPolicyInstant::from_millis(199_999),
        )],
    )
    .map_err(|_| Error::Authority)?;
    let objective = "Prepare a public Wikipedia search without submitting or navigating. Fill the search with exactly Quasar browser and choose Deutsch in the search language selector, in either order. Once both are verified, refine the search text to exactly Quasar open source browser. Use one local_write act action per turn. Locate option references when needed. For each action use mutation_quiet=100 ms, settle_budget=2000 ms, and exact value or exact selected-option verification. Do not click links or submit. The host checks the exact milestones and stops when the final prepared search is verified.";
    let objective = if let Some(site) = site {
        site.objective()
    } else if inspecting {
        "Prepare a public Wikipedia search without submitting or navigating. First inspect the existing initial baseline with read to discover the collapsed search selector's option labels and source refs. From that evidence identify Deutsch and its combobox; do not substitute the prominent Deutsch edition link for a selector option. Then fill the search with exactly Quasar browser and choose Deutsch in either order. Once both are verified, refine the search to exactly Quasar open source browser. Use one local_write act action per turn, mutation_quiet=100 ms, settle_budget=2000 ms, and exact value or selected-option verification. Locate only if the bounded read omitted a needed ref. Do not click links or submit. Rust independently checks the goals and final value."
    } else if subtree {
        "Prepare a public Wikipedia search without submitting or navigating. Fill the search with exactly Quasar browser and choose Deutsch in the search language selector, in either order. Once both are verified, refine the search text to exactly Quasar open source browser. Use one local_write act action per turn; locate option references when needed. Each action must use mutation_quiet=100 ms, settle_budget=2000 ms, and exact value or exact selected-option verification. Do not click links or submit. After all three milestones are verified, call extract with subtree scope targeting the current search text field's opaque ref and trusted schema 1. The host will freshly read only that subtree. Return prepared_query as the complete exact current search field value with its exact value-preview citation. Do not use initial extraction scope, extract early or perform further actions after the final query is verified."
    } else if combined {
        "Prepare a public Wikipedia search without submitting or navigating. Fill the search with exactly Quasar browser and choose Deutsch in the search language selector, in either order. Once both are verified, refine the search text to exactly Quasar open source browser. Use one local_write act action per turn; locate option references when needed. Each action must use mutation_quiet=100 ms, settle_budget=2000 ms, and exact value or exact selected-option verification. Do not click links or submit. After all three milestones are verified, call extract with initial scope and trusted schema 1. Return prepared_query as the complete exact current search field value with its exact value-preview citation. Do not extract early or perform further actions after the final query is verified."
    } else if extraction {
        "Extract an inventory of the ten prominent Wikipedia language-edition links. Use the trusted schema 1, initial scope. Return language_links as ten complete, exact accessible link names copied from the delivered evidence, including article-count text where it is part of a name. Do not paraphrase, translate, truncate or invent link names. Cite the exact source fragment for each item. Do not navigate or modify anything."
    } else {
        objective
    };
    let input = AgentWorkRunInput::try_new(
        manifest,
        AgentPlanLeaseBinding::new(AgentPlanLeaseId::generate(), node),
        AgentWorkContextSpec::try_new(
            context,
            if durable {
                ContextProfileStorageClass::Durable
            } else {
                ContextProfileStorageClass::Ephemeral
            },
            ContextNavigationTarget::parse(target).map_err(|_| Error::Authority)?,
        )
        .map_err(|_| Error::Authority)?,
        objective.to_owned(),
        AgentWorkRunSettings::new(
            AgentBrowserModel::Luna,
            super::probe_controller_ids()?,
            Arc::new(Clock(started)),
            started + Duration::from_secs(150),
        ),
    )
    .map_err(|_| Error::Authority)?;
    let input = if durable {
        input
            .persist_extraction_result()
            .map_err(|_| Error::Authority)?
    } else {
        input
    };
    let task: Box<dyn AgentWorkTask> = if let Some(site) = site {
        site.task(context).map_err(|_| Error::Authority)?
    } else if combined {
        Box::new(PublicPreparedResultTask {
            actions: form_task,
            extraction: AgentWorkExtractionTask::try_new(
                vec![
                    SemanticExtractionFieldSchema::try_text("prepared_query".into(), true, 64)
                        .map_err(|_| Error::Authority)?,
                ],
                AgentAccountScope::Anonymous,
            )
            .map_err(|_| Error::Authority)?,
            ready: None,
            subtree,
        })
    } else if extraction {
        Box::new(PublicExtractionTask(
            AgentWorkExtractionTask::try_new(
                vec![SemanticExtractionFieldSchema::try_text_list(
                    "language_links".into(),
                    true,
                    10,
                    256,
                )
                .map_err(|_| Error::Authority)?],
                AgentAccountScope::Anonymous,
            )
            .map_err(|_| Error::Authority)?,
        ))
    } else {
        Box::new(form_task)
    };
    Ok((profile, input, task))
}

pub(super) fn run() -> Result<(), super::ProbeFailure> {
    use super::ProbeFailure as Error;
    let started = Instant::now();
    let (profile, input, task) = input(started)?;
    let data = tempfile::tempdir().map_err(|_| Error::Runtime)?;
    let store =
        Arc::new(zephium_store::SqliteStore::open(data.path()).map_err(|_| Error::Runtime)?);
    let credential = load_macos_probe_openai_credential().map_err(|_| Error::Keychain)?;
    let transport = AgentProviderTransport::try_new(AgentProviderTransportConfig::STANDARD)
        .map_err(|_| Error::Runtime)?;
    let (controller, mut handle) = AgentWorkController::try_new_for_probe(
        input,
        transport,
        credential,
        store.clone(),
        task,
        AgentBrowserRetention::InspectablePublicData,
    )
    .map_err(|_| Error::Authority)?;
    let pending = PendingAgentRuntime::spawn_suspended_with_controller(
        AgentRuntimeConfig::STANDARD,
        Box::new(controller),
    )
    .map_err(|_| Error::Runtime)?;
    let sink = pending.native_event_sink();
    let result = zephium_engine::run_macos_agentic_work_actor_probe(
        profile,
        move |event| {
            let _ = sink.publish(event);
        },
        move |port| {
            let (runtime, completion, lifecycle) = pending.bind_browser_port(port).into_parts();
            let mut lifecycle = Some(lifecycle);
            runtime.start_run().map_err(|_| "actor_start")?;
            Ok(Box::new(move |native_failed| {
                if native_failed {
                    runtime.cancel_and_seal();
                }
                while let Some(event) = handle.take_event() {
                    let mut output = std::io::stdout().lock();
                    let result = match event.kind() {
                    AgentWorkEventKind::ModelSettled { call, input_tokens, output_tokens, request_bytes, semantic_bytes, cost_micro_usd, accounting, elapsed_millis, .. } => writeln!(output, "work-actor-turn: call={}; input_tokens={input_tokens}; output_tokens={output_tokens}; request_bytes={request_bytes}; semantic_bytes={semantic_bytes}; cost_micro_usd={cost_micro_usd}; accounting={accounting:?}; turn_ms={elapsed_millis}; wall_ms={}; content=redacted", call.get(), event.elapsed_millis()),
                    kind => writeln!(output, "work-actor-event: sequence={}; phase={kind:?}; wall_ms={}; content=redacted", event.sequence(), event.elapsed_millis()),
                };
                    if result.is_err() {
                        runtime.cancel_and_seal();
                        return Some(Err("actor_output"));
                    }
                }
                if !completion.is_stopped() {
                    return None;
                }
                let outcome = handle.take_outcome();
                let native = lifecycle
                    .take()?
                    .shutdown_until(Instant::now() + Duration::from_secs(2));
                Some(match (outcome, native) {
                (Some(AgentWorkOutcome::Succeeded(settlement)), AgentBrowserShutdownOutcome::Clean(_)) => {
                    if writeln!(std::io::stdout().lock(), "work-actor-terminal: state=succeeded; model_calls={}; verified_effects={}; lifecycle=clean; content=redacted", settlement.closure().model_calls(), settlement.closure().effects()).is_err() { Err("actor_output") } else { Ok(()) }
                }
                (Some(AgentWorkOutcome::Recovery(recovery)), _) => {
                    let _ = writeln!(std::io::stdout().lock(), "work-actor-recovery: reason={:?}; retained_callbacks={}; content=redacted", recovery.failure(), recovery.retained_callbacks());
                    Err("actor_recovery")
                }
                _ => Err("actor_terminal_unproven"),
            })
            }))
        },
    );
    let store_closed = store.shutdown_until(Instant::now() + Duration::from_secs(2));
    result.map_err(Error::Engine)?;
    if store_closed != zephium_core::ports::store::StoreShutdownOutcome::Clean {
        return Err(Error::Engine("actor_store_teardown"));
    }
    writeln!(std::io::stdout().lock(), "work-actor-qualified: model=gpt-5.6-luna; store=durable; native=production_port; focus_isolation=passed; elapsed_ms={}; content=redacted", started.elapsed().as_millis()).map_err(|_| Error::Output)?;
    Ok(())
}
