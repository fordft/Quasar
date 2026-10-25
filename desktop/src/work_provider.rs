//! Trusted capability-provider composition. This module is dormant until an
//! admitted user operation; no model selects credentials, configuration or ports.
use std::sync::Arc;
use zephium_core::{ids::ProfileId, work::WorkError};
use zephium_ipc::work::{WorkOperationStateV1, WorkOperationV1};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use zephium_ipc::work::{WorkReplyV1, WorkResponseV1};

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[path = "work_lead_run.rs"]
mod lead;

pub(crate) struct WorkProviders {
    pub(crate) activity: super::work_activity::WorkActivity,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) browser: zephium_work_composition::NativeWorkComposition,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    engine: Arc<zephium_engine::WebviewEngine>,
}
impl WorkProviders {
    pub(crate) fn new(
        engine: Arc<zephium_engine::WebviewEngine>,
        store: Arc<zephium_store::SqliteStore>,
        frames: Option<Arc<zephium_store::WorkFrameStore>>,
    ) -> Self {
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = (engine, store);
        // The page lane and searches log their refusals as closed facts.
        zephium_app::work_trace::install(Arc::new(record_diagnostic));
        zephium_app::work_trace::install_record_media(Arc::new(|event| {
            record_diagnostic(format_args!(
                "work: phase=record_media detail={event:?}; content=redacted"
            ));
        }));
        record_diagnostic(format_args!(
            "work: event=session_started build={} content=redacted",
            option_env!("ZEPHIUM_BUILD_REVISION").unwrap_or("unknown")
        ));
        // A request may offer a grant for a signed-in attached tab: the engine
        // answers whether the profile holds cookies for a site, nothing more.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let engine = engine.clone();
            let loads = engine.clone();
            zephium_app::work_context::install_session_presence(Arc::new(move |profile, hosts| {
                engine.work_sessions_present(profile, hosts)
            }));
            zephium_app::work_context::install_site_loads(Arc::new(move |profile, site| {
                loads.work_site_loads(profile, site)
            }));
        }
        Self {
            activity: super::work_activity::WorkActivity::new(frames),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            browser: zephium_work_composition::NativeWorkComposition::new(engine.clone(), store),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            engine,
        }
    }

    /// Empties Work's own website data for the profile; its answer is
    /// whether the store is empty afterwards.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) fn clear_website_data(&self, profile: ProfileId) -> std::sync::mpsc::Receiver<bool> {
        self.engine.clear_work_website_data(profile)
    }

    pub(crate) async fn run(
        &self,
        shell: zephium_app::Handle,
        profile: ProfileId,
        input: WorkOperationV1,
    ) -> WorkOperationStateV1 {
        let work = input.work();
        let phase = match &input {
            WorkOperationV1::ReadPublic { .. } => "public_read",
            WorkOperationV1::Run { .. } => "run",
            WorkOperationV1::Plan { .. } => "plan",
            WorkOperationV1::PreparePlan { .. } => "prepare_plan",
            WorkOperationV1::Prepare { .. } => "prepare",
            WorkOperationV1::PrepareAccount { .. } => "prepare_account",
            WorkOperationV1::Start { .. } => "execute",
        };
        let started = std::time::Instant::now();
        record_diagnostic(format_args!(
            "work: work={work} phase={phase} state=admitted"
        ));
        let state = match self.try_run(shell, profile, input).await {
            Ok(state) => state,
            Err(error) => WorkOperationStateV1::Refused {
                error: error.into(),
            },
        };
        let (outcome, error) = match &state {
            WorkOperationStateV1::Planned { .. } => ("planning_response", None),
            WorkOperationStateV1::Settled { .. } => ("settled_response", None),
            WorkOperationStateV1::Refused { error } => ("refused", Some(error)),
            WorkOperationStateV1::Pending { .. } => ("pending", None),
            WorkOperationStateV1::Unknown => ("unknown", None),
        };
        record_diagnostic(format_args!(
            "work: work={work} phase={phase} state={outcome} error={error:?} elapsed_ms={}",
            started.elapsed().as_millis()
        ));
        if let WorkOperationStateV1::Planned { response } = &state {
            if let Some(usage) = &response.usage {
                record_diagnostic(format_args!(
                    "work: work={work} phase={phase} input_tokens={} output_tokens={} cost_ceiling_micro_usd={}",
                    usage.input_tokens, usage.output_tokens, usage.cost_ceiling_micro_usd
                ));
            }
        }
        state
    }

    async fn try_run(
        &self,
        shell: zephium_app::Handle,
        profile: ProfileId,
        input: WorkOperationV1,
    ) -> Result<WorkOperationStateV1, WorkError> {
        match input {
            WorkOperationV1::Run {
                command,
                context,
                signed_in,
            } => {
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    let _ = (shell, command, context, signed_in);
                    Err(WorkError::Unavailable)
                }
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                {
                    let zephium_core::work::runtime::WorkRuntimeIntent::BeginAgent {
                        grant, ..
                    } = &command.intent
                    else {
                        return Err(WorkError::Invalid);
                    };
                    if command.version != 1 {
                        return Err(WorkError::Invalid);
                    }
                    grant.validate()?;
                    // Retired: sessions are decided per site inside the run.
                    let _ = signed_in;
                    let Some(models) = lead::models(profile, Some(command.work)).await else {
                        record_diagnostic(format_args!("work: phase=lead state=no_model"));
                        return Err(WorkError::Unavailable);
                    };
                    self.lead(shell, profile, command, context, models).await
                }
            }
            // The earlier runtime's public read, planning, review and start
            // are retired: every request runs as the lead.
            WorkOperationV1::ReadPublic { .. }
            | WorkOperationV1::Plan { .. }
            | WorkOperationV1::PreparePlan { .. }
            | WorkOperationV1::Prepare { .. }
            | WorkOperationV1::Start { .. } => {
                let _ = (shell, profile);
                Err(WorkError::Unavailable)
            }
            // Retired with origin grants.
            WorkOperationV1::PrepareAccount { .. } => Err(WorkError::Invalid),
        }
    }
}

// Only this module's closed facts enter the development log; generic application
// diagnostics may contain page/provider text and are deliberately excluded.
#[cfg(feature = "work-development-traces")]
fn record_browser_diagnostic(event: zephium_agent_controller::AgentWorkEventKind) {
    use zephium_agent_controller::AgentWorkEventKind;
    match event {
        AgentWorkEventKind::DecisionSettled(fact) => {
            record_diagnostic(format_args!(
                "work: phase=decision backend={:?} model={} questions={} state_bytes={} input_tokens={:?} elapsed_ms={} http_status={:?} attempts={:?} failure={:?} envelope={:?}",
                fact.backend, fact.model, fact.question_count, fact.state_bytes,
                fact.input_tokens, fact.elapsed_millis, fact.http_status, fact.attempts,
                fact.failure, fact.envelope_failure,
            ));
            if let Some(facts) = fact.rejected_envelope {
                record_diagnostic(format_args!(
                    "work: phase=decision_rejected_envelope facts={facts:?}"
                ));
            }
            for (kind, counts) in ["noul", "choice", "score"]
                .into_iter()
                .zip(fact.confidence_buckets)
            {
                record_diagnostic(format_args!(
                    "work: phase=decision_confidence kind={kind} below_080={} below_095={} below_098={} at_least_098={}",
                    counts[0], counts[1], counts[2], counts[3],
                ));
            }
        }
        AgentWorkEventKind::DecisionFallback {
            counts,
            purposes,
            capacity,
        } => {
            for (kind, counts) in ["noul", "choice", "score"].into_iter().zip(counts) {
                record_diagnostic(format_args!(
                    "work: phase=decision_fallback kind={kind} capacity={capacity} unavailable={} rate_limited={} invalid_answer={} low_confidence={}",
                    counts[0], counts[1], counts[2], counts[3],
                ));
            }
            // Declaration order of DecisionPurpose; a slot past it is still printed.
            const PURPOSES: [&str; 10] = [
                "challenge",
                "action",
                "locate",
                "picture",
                "relevance",
                "wall",
                "completion",
                "score",
                "navigation",
                "evidence",
            ];
            let mut line = String::new();
            for (index, count) in purposes.iter().enumerate() {
                use std::fmt::Write;
                let _ = match PURPOSES.get(index) {
                    Some(purpose) => write!(line, " {purpose}={count}"),
                    None => write!(line, " purpose_{index}={count}"),
                };
            }
            record_diagnostic(format_args!("work: phase=decision_fallback_purpose{line}"));
        }
        event => record_diagnostic(format_args!("work: phase=agent_browser event={event:?}")),
    }
}

pub(crate) fn record_diagnostic(arguments: std::fmt::Arguments<'_>) {
    #[cfg(feature = "work-development-traces")]
    super::work_diagnostics::record(arguments);
    #[cfg(not(feature = "work-development-traces"))]
    super::write_diagnostic(arguments);
}
