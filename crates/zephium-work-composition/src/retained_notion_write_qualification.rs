//! Release-excluded authenticated, reversible Notion write qualification.
//! The trusted task admits only two exact title transitions on one exact page;
//! it grants no click, submit, navigation, arbitrary text or script authority.

use crate::retained_notion_support::{load_parts, ORIGIN};
use std::io::Write as _;
use zephium_agent_controller::*;
use zephium_agentic::*;
use zephium_app::{RetainedWorkHandle, RetainedWorkPhase};

const MAX_MODEL_CALLS: u8 = 6;
const ORIGINAL_TITLE: &str = "Quasar Agent Qualification";
const TEMPORARY_TITLE: &str = "Quasar Agent Qualification — Write Probe";
pub const OBJECTIVE: &str = "Temporarily change the current signed-in Notion page title from exactly `Quasar Agent Qualification` to exactly `Quasar Agent Qualification — Write Probe`, then restore it to exactly `Quasar Agent Qualification`. Use the current semantic field reference; do not invent selectors. Perform exactly one bounded fill action per turn with effect=external_write, wait=mutation_quiet 1000 ms, verification=target_value_matches_input, and settle_budget=5000 ms. Continue after the first verified change and restore the original title. After the host verifies the restoration from fresh browser state, call extract with trusted schema 1 and return restored_page_title as the exact current title field value with only its value-preview source. Do not navigate, click, submit, edit any other field, open another page, or run code. Both exact remote updates are explicitly approved for this disposable qualification workspace; the host independently restricts the targets, values, ordering, account, origin, and effects.";

pub fn configuration_diagnostic() -> String {
    "work-retained-notion-write-config: entry=admit_retained_trusted_work provider=OpenAIResponses model=gpt-5.6-luna retention=inspectable-synthetic-authenticated task=notion-title-restore-v1 effect=approved_external_write account=user_attested".into()
}

pub fn load_request(
    started: std::time::Instant,
    profile: zephium_app::AgentWorkProfileBinding,
) -> Result<crate::TrustedWorkRequest, &'static str> {
    let parts = load_parts(
        started,
        profile,
        OBJECTIVE,
        &[
            SemanticEffectClass::Read,
            SemanticEffectClass::ExternalWrite,
        ],
        10,
        MAX_MODEL_CALLS,
    )?;
    let task = NotionWriteTask::try_new(parts.identity, parts.origin.clone(), parts.account)
        .map_err(|_| "task")?;
    Ok(parts.finish(Box::new(task)))
}

#[derive(Clone)]
struct Checkpoint {
    observation: SemanticObservationId,
    generation: SemanticObservationGeneration,
    frame: SemanticFrameJoin,
    invocation: SemanticInvocationId,
    snapshot: SemanticSnapshotGeneration,
    reference: SemanticReferenceId,
}

struct NotionWriteTask {
    inner: AgentWorkFormExtractionTask,
    identity: ContextIdentity,
    origin: SemanticOrigin,
    initial: Option<Checkpoint>,
    changed: Option<Checkpoint>,
    restored: Option<Checkpoint>,
}

impl NotionWriteTask {
    fn try_new(
        identity: ContextIdentity,
        origin: SemanticOrigin,
        account: AgentAccountScope,
    ) -> Result<Self, AgentWorkFailure> {
        if identity.kind() != ContextKind::Owned
            || !matches!(account, AgentAccountScope::Authenticated(_))
        {
            return Err(AgentWorkFailure::Contract);
        }
        let transition = |from: &str, to: &str| {
            AgentWorkFormPhase::try_new(vec![AgentWorkFormGoal::fill_transition(
                None,
                from.into(),
                to.into(),
            )?])
        };
        let inner = AgentWorkFormTask::try_new_external_update(
            identity,
            origin.clone(),
            account,
            vec![
                transition(ORIGINAL_TITLE, TEMPORARY_TITLE)?,
                transition(TEMPORARY_TITLE, ORIGINAL_TITLE)?,
            ],
        )?
        .with_baseline_read()
        .with_extraction(vec![SemanticExtractionFieldSchema::try_text(
            "restored_page_title".into(),
            true,
            256,
        )
        .map_err(|_| AgentWorkFailure::Contract)?])?;
        Ok(Self {
            inner,
            identity,
            origin,
            initial: None,
            changed: None,
            restored: None,
        })
    }

    fn checkpoint(
        &self,
        observation: &SemanticObservation,
        stage: &'static str,
        expected: &str,
    ) -> Result<Checkpoint, AgentWorkFailure> {
        let (checkpoint, readiness) = self.title_state(observation, stage, expected)?;
        if readiness != AgentWorkInitialReadiness::Ready {
            return Err(AgentWorkFailure::Contract);
        }
        Ok(checkpoint)
    }

    fn title_state(
        &self,
        observation: &SemanticObservation,
        stage: &'static str,
        expected: &str,
    ) -> Result<(Checkpoint, AgentWorkInitialReadiness), AgentWorkFailure> {
        let context = observation.request().context();
        let [frame] = observation.frames() else {
            return Err(AgentWorkFailure::Contract);
        };
        if context.identity() != self.identity
            || context.frame() != FrameId::MAIN
            || !matches!(observation.request().scope(), SemanticScope::Initial)
            || observation.request().generation() != SemanticObservationGeneration::INITIAL
            || frame.frame().context() != context
            || frame.frame().frame() != FrameId::MAIN
            || frame.frame().origin() != &self.origin
            || frame.completeness() != SemanticCompleteness::Complete
            || observation.frame_boundaries().iter().any(|boundary| {
                boundary.parent_frame() != FrameId::MAIN
                    || boundary.status()
                        != SemanticFrameBoundaryStatus::Unsupported(
                            SemanticFrameUnsupported::PolicyBlocked,
                        )
            })
        {
            return Err(AgentWorkFailure::Contract);
        }
        let mut textboxes = 0_u16;
        let mut exact_values = 0_u16;
        let mut candidates = 0_u16;
        let mut candidate = None;
        for node in frame.nodes() {
            if node.role() != SemanticRole::Textbox {
                continue;
            }
            textboxes = textboxes.saturating_add(1);
            let exact = matches!(node.value(), Some(SemanticValueSummary::Text(value)) if !value.preview().truncated() && value.preview().source_bytes() == expected.len() && value.preview().text() == expected);
            if !exact {
                continue;
            }
            exact_values = exact_values.saturating_add(1);
            if node.sensitivity() != SemanticSensitivity::Public {
                return Err(AgentWorkFailure::Contract);
            }
            candidate = Some(node);
            if !node.states().contains(SemanticState::Disabled)
                && node.operations().contains(SemanticOperationClass::Fill)
            {
                candidates = candidates.saturating_add(1);
            }
        }
        writeln!(std::io::stdout().lock(), "work-retained-notion-write-observation: stage={stage} nodes={} boundaries={} completeness={:?} textboxes={textboxes} exact_values={exact_values} eligible_candidates={candidates} content=redacted", observation.node_count(), observation.frame_boundaries().len(), frame.completeness()).map_err(|_| AgentWorkFailure::Contract)?;
        if exact_values != 1 {
            return Err(AgentWorkFailure::Contract);
        }
        let node = candidate.ok_or(AgentWorkFailure::Contract)?;
        if let Some(shape) = node.editable_structure() {
            writeln!(std::io::stdout().lock(), "work-retained-notion-write-structure: stage={stage} child_count={} truncated={} text={} elements={} other={} editable_parent={} content=redacted", shape.child_count(), shape.truncated(), shape.has_text(), shape.has_elements(), shape.has_other(), shape.editable_parent()).map_err(|_| AgentWorkFailure::Contract)?;
        }
        writeln!(
            std::io::stdout().lock(),
            "work-retained-notion-write-support: stage={stage} support={:?} content=redacted",
            node.fill_support()
        )
        .map_err(|_| AgentWorkFailure::Contract)?;
        let index = frame
            .nodes()
            .iter()
            .position(|entry| entry.reference() == node.reference())
            .and_then(|index| u16::try_from(index).ok())
            .ok_or(AgentWorkFailure::Contract)?;
        if frame
            .nodes()
            .iter()
            .any(|entry| entry.parent() == Some(index))
        {
            return Err(AgentWorkFailure::Contract);
        }
        Ok((
            Checkpoint {
                observation: observation.request().id(),
                generation: observation.request().generation(),
                frame: frame.frame().clone(),
                invocation: frame.invocation(),
                snapshot: frame.generation(),
                reference: node.reference(),
            },
            if candidates == 1 {
                AgentWorkInitialReadiness::Ready
            } else {
                AgentWorkInitialReadiness::Pending
            },
        ))
    }

    fn fresh_successor(prior: &Checkpoint, next: &Checkpoint) -> bool {
        prior.frame == next.frame
            && prior.observation != next.observation
            && prior.invocation != next.invocation
            && next.snapshot > prior.snapshot
    }
}

impl AgentWorkTask for NotionWriteTask {
    fn initial_readiness(
        &self,
        observation: &SemanticObservation,
    ) -> Result<AgentWorkInitialReadiness, AgentWorkFailure> {
        if self.initial.is_some() || self.changed.is_some() || self.restored.is_some() {
            return Err(AgentWorkFailure::Contract);
        }
        // A no-Fill snapshot is not proof of eventual support. It permits only
        // the controller's finite read-only startup wait. Rich/readonly hosts
        // that never become natively supported time out without model/effects.
        self.title_state(observation, "readiness", ORIGINAL_TITLE)
            .map(|(_, readiness)| readiness)
    }

    fn allows_actions_before_extraction(&self) -> bool {
        true
    }

    fn allows_baseline_read(&self) -> bool {
        self.inner.allows_baseline_read()
    }

    fn extraction_schema(&self) -> Option<&SemanticExtractionSchema> {
        self.inner.extraction_schema()
    }

    fn model_action_operations(
        &self,
        node: &SemanticNode,
        observation: &SemanticObservation,
    ) -> Result<SemanticOperations, AgentWorkFailure> {
        self.inner.model_action_operations(node, observation)
    }

    fn evaluate(
        &mut self,
        observation: &SemanticObservation,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        if self.restored.is_some() {
            return Err(AgentWorkFailure::Contract);
        }
        let expected = if self.initial.is_none() {
            ORIGINAL_TITLE
        } else if self.changed.is_none() {
            TEMPORARY_TITLE
        } else {
            ORIGINAL_TITLE
        };
        let stage = if self.initial.is_none() {
            "initial"
        } else if self.changed.is_none() {
            "changed"
        } else {
            "restored"
        };
        let checkpoint = self.checkpoint(observation, stage, expected)?;
        let progress = self.inner.evaluate(observation)?;
        if self.initial.is_none() {
            if progress != AgentWorkTaskProgress::Continue {
                return Err(AgentWorkFailure::Contract);
            }
            self.initial = Some(checkpoint);
        } else if self.changed.is_none() {
            if progress != AgentWorkTaskProgress::Continue
                || !Self::fresh_successor(
                    self.initial.as_ref().ok_or(AgentWorkFailure::Contract)?,
                    &checkpoint,
                )
            {
                return Err(AgentWorkFailure::Contract);
            }
            self.changed = Some(checkpoint);
        } else {
            if progress != AgentWorkTaskProgress::ReadyForExtraction
                || !Self::fresh_successor(
                    self.changed.as_ref().ok_or(AgentWorkFailure::Contract)?,
                    &checkpoint,
                )
            {
                return Err(AgentWorkFailure::Contract);
            }
            self.restored = Some(checkpoint);
        }
        Ok(progress)
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

    fn accept_extraction(
        &mut self,
        result: &SemanticExtractionResult<'_>,
    ) -> Result<AgentWorkTaskProgress, AgentWorkFailure> {
        let expected = self.restored.as_ref().ok_or(AgentWorkFailure::Contract)?;
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
        if field.name() != "restored_page_title"
            || value.as_str() != ORIGINAL_TITLE
            || result.observation() != expected.observation
            || provenance.observation() != expected.observation
            || provenance.observation_generation() != expected.generation
            || provenance.frame() != &expected.frame
            || provenance.invocation() != expected.invocation
            || provenance.snapshot() != expected.snapshot
            || provenance.reference() != expected.reference
            || fragment.role() != SemanticRole::Textbox
            || fragment.field() != SemanticReadField::TextValue
            || !matches!(fragment.content(), SemanticReadContent::ValuePreview(preview) if preview.text() == ORIGINAL_TITLE && !preview.truncated() && preview.source_bytes() == ORIGINAL_TITLE.len())
        {
            return Err(AgentWorkFailure::Contract);
        }
        self.inner.accept_extraction(result)
    }
}

pub fn cancel(view: &RetainedWorkHandle) -> bool {
    view.stop()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ApplicationReport {
    pub accepted: bool,
    pub model_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_micro_usd: u64,
    pub reads: u64,
    pub proposed_actions: u8,
    pub active_actions: u8,
    pub verified_actions: u8,
    pub source_mapping_verified: bool,
    pub durable_terminal_verified: bool,
}

#[derive(Default)]
pub struct ApplicationObserver {
    report: ApplicationReport,
    sequence: u64,
    extracts: u8,
    failed: bool,
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
                    match total.checked_add(delta) {
                        Some(sum) => *total = sum,
                        None => self.failed = true,
                    }
                }
            }
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Read) => {
                self.report.reads = self.report.reads.saturating_add(1);
                self.failed |= self.report.reads > 2;
            }
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Act) => {
                self.report.proposed_actions = self.report.proposed_actions.saturating_add(1);
                self.failed |= self.report.proposed_actions > 2
                    || self.report.active_actions + 1 != self.report.proposed_actions
                    || self.report.verified_actions != self.report.active_actions;
            }
            AgentWorkEventKind::ActionActive => {
                self.report.active_actions = self.report.active_actions.saturating_add(1);
                self.failed |= self.report.active_actions != self.report.proposed_actions
                    || self.report.verified_actions + 1 != self.report.active_actions;
            }
            AgentWorkEventKind::Verified => {
                self.report.verified_actions = self.report.verified_actions.saturating_add(1);
                self.failed |= self.report.verified_actions != self.report.active_actions
                    || self.report.active_actions != self.report.proposed_actions;
            }
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Extract) => {
                self.extracts = self.extracts.saturating_add(1);
                self.failed |= self.extracts != 1 || !self.actions_complete();
            }
            AgentWorkEventKind::ToolProposed(_)
            | AgentWorkEventKind::InspectionRefused
            | AgentWorkEventKind::InspectionAnchorLost
            | AgentWorkEventKind::NeedsHuman(_)
            | AgentWorkEventKind::Recovery => self.failed = true,
            _ => {}
        }
    }

    fn actions_complete(&self) -> bool {
        !self.failed
            && self.report.proposed_actions == 2
            && self.report.active_actions == 2
            && self.report.verified_actions == 2
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
            self.failed |= writeln!(std::io::stdout().lock(), "work-retained-notion-write-event: sequence={} phase={:?} wall_ms={} content=redacted", event.sequence(), event.kind(), event.elapsed_millis()).is_err();
        }
        if !matches!(
            snapshot.phase,
            RetainedWorkPhase::Terminal | RetainedWorkPhase::Uncertain | RetainedWorkPhase::Refused
        ) {
            return None;
        }
        let resource_failure = resource_failure();
        self.report.durable_terminal_verified = snapshot.record.is_some_and(|record| {
            record.disposition() == AgentWorkDisposition::Succeeded
                && record.debt() == AgentWorkDebt::NONE
                && record.key()[16..] == snapshot.run.bytes()
        });
        self.report.source_mapping_verified = view
            .take_extraction()
            .is_some_and(|result| verify_owned(&result) && view.take_extraction().is_none());
        self.report.accepted = self.actions_complete()
            && self.extracts == 1
            && resource_failure.is_none()
            && snapshot.phase == RetainedWorkPhase::Terminal
            && snapshot.failure.is_none()
            && snapshot.persistence_failure.is_none()
            && self.report.durable_terminal_verified
            && self.report.source_mapping_verified
            && (3..=u64::from(MAX_MODEL_CALLS)).contains(&self.report.model_calls)
            && self
                .report
                .input_tokens
                .saturating_add(self.report.output_tokens)
                <= 100_000
            && self.report.cost_micro_usd <= 100_000;
        let semantic_restoration_verified = self.actions_complete();
        self.report.accepted &= writeln!(std::io::stdout().lock(), "work-retained-notion-write-terminal: phase={:?} failure={:?} persistence_failure={:?} resource_failure={resource_failure:?} proposed={} active={} verified={} extracts={} source_mapping={} durable={} semantic_restoration_verified={semantic_restoration_verified} remote_persistence=not_verified accepted={} content=redacted", snapshot.phase, snapshot.failure, snapshot.persistence_failure, self.report.proposed_actions, self.report.active_actions, self.report.verified_actions, self.extracts, self.report.source_mapping_verified, self.report.durable_terminal_verified, self.report.accepted).is_ok();
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
        && field.name() == "restored_page_title"
        && value.as_str() == ORIGINAL_TITLE
        && sources.next().is_none()
        && source.observation == result.observation()
        && source.observation_generation == result.observation_generation()
        && source.frame.frame() == FrameId::MAIN
        && SemanticOrigin::parse(ORIGIN).as_ref() == Ok(source.frame.origin())
        && source.role == SemanticRole::Textbox
        && source.field == SemanticReadField::TextValue
        && source.sensitivity == SemanticSensitivity::Public
        && matches!(&source.content, SemanticOwnedReadContent::ValuePreview { text, source_bytes, truncated } if text == ORIGINAL_TITLE && *source_bytes == ORIGINAL_TITLE.len() && !truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> ContextJoin {
        let identity = ContextIdentity::new(
            ContextId::generate(),
            ContextRunId::generate(),
            zephium_core::ids::ProfileId::generate(),
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
        registry.join(identity.id()).unwrap()
    }

    fn observation(context: ContextJoin, id: u64, title: &str) -> SemanticObservation {
        observation_with_extra(context, id, title, "")
    }

    fn observation_with_extra(
        context: ContextJoin,
        id: u64,
        title: &str,
        extra: &str,
    ) -> SemanticObservation {
        observation_with_operations(context, id, title, extra, 3)
    }

    fn observation_with_operations(
        context: ContextJoin,
        id: u64,
        title: &str,
        extra: &str,
        operations: u8,
    ) -> SemanticObservation {
        let frame = SemanticFrameJoin::try_new(
            context,
            FrameId::MAIN,
            context.frame_generation(),
            SemanticOrigin::parse(ORIGIN).unwrap(),
            SemanticFrameTrust::SameOrigin,
        )
        .unwrap();
        let wire = format!(
            r#"{{"v":1,"i":{id},"g":{id},"c":"complete","n":[{{"k":1,"r":"document","o":16}},{{"k":2,"p":0,"r":"textbox","n":"page title","s":64,"o":{operations},"v":{{"k":"text","value":"{title}"}}}}{extra}]}}"#
        );
        let snapshot = decode_semantic_snapshot(
            SemanticDecodeContext::new(
                SemanticInvocationId::new(id).unwrap(),
                frame,
                SemanticSnapshotGeneration::new(id).unwrap(),
            ),
            wire.as_bytes(),
        )
        .unwrap();
        SemanticObservationAssembler::new(
            SemanticObservationRequest::initial(
                SemanticObservationId::new(id).unwrap(),
                context,
                SemanticObservationBudget::INITIAL_FILTERED,
            ),
            snapshot,
        )
        .unwrap()
        .finish()
        .unwrap()
    }

    fn fill(observation: &SemanticObservation, value: &str) -> SemanticPreparedAction {
        let proposal = SemanticActionProposal::try_new(
            SemanticActionIntent::Fill {
                target: SemanticReferenceId::new(2).unwrap(),
                value: SemanticActionText::try_new(value.into()).unwrap(),
            },
            SemanticEffectClass::ExternalWrite,
            SemanticWaitCondition::MutationQuiet(
                SemanticMutationQuietPeriod::try_new(1_000).unwrap(),
            ),
            SemanticVerification::TargetValueMatchesInput,
            SemanticSettleBudget::try_new(5_000).unwrap(),
        )
        .unwrap();
        let snapshot = &observation.frames()[0];
        SemanticActionBatch::bind(
            SemanticActionBatchId::new(1).unwrap(),
            observation,
            &[snapshot.frame().clone()],
            vec![proposal],
        )
        .unwrap()
        .actions()[0]
            .prepare(snapshot)
            .unwrap()
    }

    fn mapped<'a>(
        schema: &SemanticExtractionSchema,
        observation: &'a SemanticObservation,
    ) -> SemanticExtractionResult<'a> {
        let read = read_selected_semantic_observation(
            observation,
            SemanticReadAuthority::Initial,
            SemanticCaptureInstant::from_millis(30),
            SemanticReadSensitivityLimit::PublicOnly,
            SemanticReadBudget::STANDARD,
            schema.source_roles(),
        )
        .unwrap();
        let source = read
            .fragments()
            .iter()
            .find(|fragment| fragment.field() == SemanticReadField::TextValue)
            .unwrap()
            .id()
            .get();
        let delivery = encode_semantic_read(
            &read,
            SemanticModelEncodingBudget::INITIAL_PROVIDER_EXACT_CONSERVATIVE,
        )
        .unwrap()
        .admit_conservative_utf8(
            &SemanticTokenizerRevision::try_new("notion-write-test-v1".into()).unwrap(),
        )
        .unwrap()
        .settle_delivery(SemanticModelDeliverySettlement::Committed)
        .unwrap();
        let output = format!(
            r#"{{"v":1,"schema":1,"fields":[{{"name":"restored_page_title","value":{{"k":"text","value":"{ORIGINAL_TITLE}","sources":["@r{source}"]}}}}]}}"#
        );
        extract_semantic_read(
            schema,
            &read,
            &delivery,
            SemanticReadSensitivityLimit::PublicOnly,
            output.as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn task_is_exact_authenticated_external_write_without_navigation() {
        let identity = ContextIdentity::new(
            ContextId::generate(),
            ContextRunId::generate(),
            zephium_core::ids::ProfileId::generate(),
            ContextKind::Owned,
        );
        let account = AgentAccountScope::Authenticated(AgentAccountId::generate());
        let task =
            NotionWriteTask::try_new(identity, SemanticOrigin::parse(ORIGIN).unwrap(), account)
                .unwrap();
        assert!(task.navigation_target().is_none());
        assert!(task.navigation_route().is_none());
        assert!(task.navigation_discovery().is_none());
        assert!(task.allows_baseline_read());
        assert!(task.allows_actions_before_extraction());
        assert!(!task.allows_progressive_observation());
        assert!(!task.allows_subtree_extraction());
        assert_eq!(
            task.extraction_schema()
                .unwrap()
                .fields()
                .iter()
                .map(|field| field.name())
                .collect::<Vec<_>>(),
            ["restored_page_title"]
        );
    }

    #[test]
    fn title_hydration_waits_only_for_exact_public_initial_target_without_progress() {
        let context = context();
        let mut task = NotionWriteTask::try_new(
            context.identity(),
            SemanticOrigin::parse(ORIGIN).unwrap(),
            AgentAccountScope::Authenticated(AgentAccountId::generate()),
        )
        .unwrap();
        let pending = observation_with_operations(context, 1, ORIGINAL_TITLE, "", 9);
        assert_eq!(
            task.initial_readiness(&pending).unwrap(),
            AgentWorkInitialReadiness::Pending
        );
        assert!(task.initial.is_none());
        assert!(
            task.evaluate(&pending).is_err(),
            "not usable; waiting never grants progress"
        );
        assert!(task
            .initial_readiness(&observation(context, 2, TEMPORARY_TITLE))
            .is_err());
        let duplicate = format!(
            r#",{{"k":3,"p":0,"r":"textbox","o":1,"v":{{"k":"text","value":"{ORIGINAL_TITLE}"}}}}"#
        );
        assert!(task
            .initial_readiness(&observation_with_extra(
                context,
                2,
                ORIGINAL_TITLE,
                &duplicate
            ))
            .is_err());
        assert!(task
            .initial_readiness(&observation(self::context(), 2, ORIGINAL_TITLE))
            .is_err());
        let rich = r#",{"k":3,"p":1,"r":"button","n":"nested editor control","o":1}"#;
        assert!(task
            .initial_readiness(&observation_with_operations(
                context,
                2,
                ORIGINAL_TITLE,
                rich,
                9
            ))
            .is_err());
        let ready = observation(context, 3, ORIGINAL_TITLE);
        assert_eq!(
            task.initial_readiness(&ready).unwrap(),
            AgentWorkInitialReadiness::Ready
        );
        task.evaluate(&ready).unwrap();
        assert!(
            task.initial_readiness(&ready).is_err(),
            "startup wait cannot become mutation retry"
        );
    }

    #[test]
    fn task_requires_exact_update_restore_and_fresh_source_bound_result() {
        let context = context();
        let account = AgentAccountScope::Authenticated(AgentAccountId::generate());
        let mut task = NotionWriteTask::try_new(
            context.identity(),
            SemanticOrigin::parse(ORIGIN).unwrap(),
            account,
        )
        .unwrap();
        task.attest_account(context, AgentPolicyInstant::from_millis(1))
            .unwrap();

        let initial = observation(context, 1, ORIGINAL_TITLE);
        assert_eq!(
            task.evaluate(&initial).unwrap(),
            AgentWorkTaskProgress::Continue
        );
        assert_eq!(
            task.assess(&fill(&initial, TEMPORARY_TITLE))
                .unwrap()
                .actual_effect(),
            SemanticEffectClass::ExternalWrite
        );

        let changed = observation(context, 2, TEMPORARY_TITLE);
        assert_eq!(
            task.evaluate(&changed).unwrap(),
            AgentWorkTaskProgress::Continue
        );
        assert!(task.assess(&fill(&initial, TEMPORARY_TITLE)).is_err());
        assert!(task.assess(&fill(&changed, ORIGINAL_TITLE)).is_ok());

        let restored = observation(context, 3, ORIGINAL_TITLE);
        assert_eq!(
            task.evaluate(&restored).unwrap(),
            AgentWorkTaskProgress::ReadyForExtraction
        );
        let schema = task.extraction_schema().unwrap().clone();
        assert_eq!(
            task.accept_extraction(&mapped(&schema, &restored)).unwrap(),
            AgentWorkTaskProgress::Complete
        );
    }

    #[test]
    fn task_refuses_an_unexpected_initial_remote_value() {
        let context = context();
        let account = AgentAccountScope::Authenticated(AgentAccountId::generate());
        let mut task = NotionWriteTask::try_new(
            context.identity(),
            SemanticOrigin::parse(ORIGIN).unwrap(),
            account,
        )
        .unwrap();
        assert!(task
            .evaluate(&observation(context, 1, "unexpected title"))
            .is_err());
    }

    #[test]
    fn task_refuses_a_second_exactly_valued_field() {
        let context = context();
        let account = AgentAccountScope::Authenticated(AgentAccountId::generate());
        let mut task = NotionWriteTask::try_new(
            context.identity(),
            SemanticOrigin::parse(ORIGIN).unwrap(),
            account,
        )
        .unwrap();
        let decoy = format!(
            r#",{{"k":3,"p":0,"r":"textbox","n":"Other field","s":64,"o":3,"v":{{"k":"text","value":"{ORIGINAL_TITLE}"}}}}"#
        );
        assert!(task
            .evaluate(&observation_with_extra(context, 1, ORIGINAL_TITLE, &decoy,))
            .is_err());
    }

    #[test]
    fn observer_requires_two_complete_actions_then_one_extraction() {
        let mut observer = ApplicationObserver::default();
        for event in [
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Act),
            AgentWorkEventKind::ActionActive,
            AgentWorkEventKind::Verified,
            AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Act),
            AgentWorkEventKind::ActionActive,
            AgentWorkEventKind::Verified,
        ] {
            observer.observe_kind(event);
        }
        assert!(observer.actions_complete());
        observer.observe_kind(AgentWorkEventKind::ToolProposed(
            AgentBrowserToolKind::Extract,
        ));
        assert!(observer.healthy());
        assert_eq!(observer.extracts, 1);
    }

    #[test]
    fn observer_rejects_incomplete_out_of_order_or_extra_actions() {
        for sequence in [
            vec![AgentWorkEventKind::Verified],
            vec![
                AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Act),
                AgentWorkEventKind::Verified,
            ],
            vec![AgentWorkEventKind::ToolProposed(
                AgentBrowserToolKind::Extract,
            )],
        ] {
            let mut observer = ApplicationObserver::default();
            for event in sequence {
                observer.observe_kind(event);
            }
            assert!(!observer.healthy());
        }
        let mut observer = ApplicationObserver::default();
        for _ in 0..3 {
            observer.observe_kind(AgentWorkEventKind::ToolProposed(AgentBrowserToolKind::Act));
        }
        assert!(!observer.healthy());
    }
}
