//! The lead's run: its models per role, its search backend and the page
//! machinery, all on the models the person chose. Searches open the
//! person's own search engine's results page in the agent browser and read
//! it with no model; the chosen models' own search, or the OpenAI public
//! search, answers only when that page cannot be used, a few times a run.
use super::*;
use zephium_app::work_lead::{LeadModel, WorkLeadModels, WorkLeadService};
use zephium_core::work::model::WorkModelRole;

/// The models for each role; a helper role without its own model uses the
/// lead's. `None` when no lead model resolves, and the run is refused.
pub(crate) async fn models(
    profile: ProfileId,
    work: Option<zephium_core::work::WorkId>,
) -> Option<WorkLeadModels> {
    // Hosted calls are noted against their task on this computer, for the
    // usage history; the service is never told the task.
    let source = work.map(|work| zephium_app::work_models::WorkCallSource::Task {
        profile: profile.to_string(),
        work: work.to_string(),
    });
    let resolve = |role| {
        let source = source.clone();
        async move {
            zephium_app::work_models::resolve_entry_for(profile, role, source)
                .await
                .ok()
                .map(|(entry, client)| LeadModel { entry, client })
        }
    };
    let lead = resolve(WorkModelRole::Lead).await?;
    let page = resolve(WorkModelRole::Page)
        .await
        .unwrap_or_else(|| lead.clone());
    let light = resolve(WorkModelRole::Light)
        .await
        .unwrap_or_else(|| page.clone());
    Some(WorkLeadModels { lead, page, light })
}

/// Search opens the person's own search engine's results page in the agent
/// browser, as if they typed the query in the address bar, and Rust reads
/// the results off it: no model call. A page that cannot be used falls back
/// to a provider's search a few times per run. A lead run always has the
/// agent browser, so a browser search leads unless the person's custom
/// engine has no usable address; then the provider search stands alone.
async fn search_backend(
    profile: ProfileId,
    models: &WorkLeadModels,
) -> Result<Box<dyn zephium_core::work::search::WorkPublicSearchProvider>, WorkError> {
    let fallback = provider_search(profile, models).await?;
    let (engine, template) = search_engine().await;
    let Ok(browser) = zephium_agentic::BrowserSearch::try_new(engine, &template) else {
        return Ok(match fallback {
            Some((search, _)) => search,
            None => {
                record_diagnostic(format_args!("work: phase=lead state=search backend=none"));
                Box::new(NoSearch)
            }
        });
    };
    record_diagnostic(format_args!(
        "work: phase=lead state=search backend=browser engine={} fallback={}",
        engine.id(),
        fallback
            .as_ref()
            .map_or("none", |(_, grant)| match grant.provider {
                zephium_core::work::search::WorkSearchProvider::Native => "native",
                zephium_core::work::search::WorkSearchProvider::OpenAi => "openai",
                zephium_core::work::search::WorkSearchProvider::Browser => "browser",
            })
    ));
    Ok(match fallback {
        Some((search, grant)) => Box::new(browser.with_fallback(search, grant)?),
        None => Box::new(browser),
    })
}

/// The engine the person searches with in the address bar, and their
/// custom engine's address.
async fn search_engine() -> (zephium_core::search::SearchEngine, String) {
    use zephium_core::ports::store::Store;
    tauri::async_runtime::spawn_blocking(|| {
        let store = crate::APP_STORE.get()?;
        Some((
            store
                .app_setting("search.engine")
                .as_deref()
                .and_then(zephium_core::search::SearchEngine::from_id)
                .unwrap_or_default(),
            store.app_setting("search.custom-url").unwrap_or_default(),
        ))
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// The provider search a browser search falls back to: the chosen models'
/// own search, the light model first since searches are many and short;
/// else the person's OpenAI key's fixed public search; else none.
async fn provider_search(
    _profile: ProfileId,
    models: &WorkLeadModels,
) -> Result<
    Option<(
        Box<dyn zephium_core::work::search::WorkPublicSearchProvider>,
        zephium_core::work::search::WorkSearchFallbackV1,
    )>,
    WorkError,
> {
    use zephium_core::work::search::{WorkSearchFallbackV1, WorkSearchProvider};
    for model in [&models.light, &models.lead, &models.page] {
        if let Ok(search) =
            zephium_agentic::ModelNativeSearch::try_new(model.entry.clone(), model.client.clone())
        {
            record_diagnostic(format_args!(
                "work: phase=lead state=search provider=native model={}",
                model.entry.id
            ));
            let search: Box<dyn zephium_core::work::search::WorkPublicSearchProvider> =
                Box::new(search);
            return Ok(Some((
                search,
                WorkSearchFallbackV1 {
                    provider: WorkSearchProvider::Native,
                    model: model.entry.id.clone(),
                },
            )));
        }
    }
    Ok(None)
}

/// The page agent runs on the page model the person chose; a page model that
/// cannot call tools leaves the page to the lead model.
fn page_model(
    models: &WorkLeadModels,
) -> Result<zephium_agentic::AgentProviderModelClient, WorkError> {
    for model in [&models.page, &models.lead] {
        if let Ok(page) =
            zephium_agentic::AgentProviderModelClient::try_new(&model.entry, model.client.clone())
        {
            record_diagnostic(format_args!(
                "work: phase=lead state=page_model model={} vision={} priced={}",
                model.entry.id,
                page.supports_vision(),
                page.is_priced()
            ));
            return Ok(page);
        }
    }
    record_diagnostic(format_args!("work: phase=lead state=page_model model=none"));
    Err(WorkError::Unavailable)
}

/// No search backend: every search is refused before anything is sent.
struct NoSearch;

impl zephium_core::work::search::WorkPublicSearchProvider for NoSearch {
    fn search<'a>(
        &'a self,
        _scope: &'a zephium_core::work::search::WorkPublicSearchScope,
        _context: &'a [zephium_core::work::context::WorkContextBody],
        _limits: zephium_core::work::runtime::WorkExecutionLimits,
    ) -> zephium_core::work::search::WorkPublicSearchFuture<'a> {
        Box::pin(async {
            Err(
                zephium_core::work::search::WorkPublicSearchError::NotDispatched(
                    WorkError::Unavailable,
                ),
            )
        })
    }
}

impl WorkProviders {
    pub(super) async fn lead(
        &self,
        shell: zephium_app::Handle,
        profile: ProfileId,
        command: zephium_ipc::work::WorkCommandV1,
        context: Option<zephium_core::work::context::WorkContextSelectionV1>,
        models: WorkLeadModels,
    ) -> Result<WorkOperationStateV1, WorkError> {
        use zephium_agentic::AgentProviderTransportConfig;
        if !matches!(
            command.intent,
            zephium_core::work::runtime::WorkRuntimeIntent::BeginAgent { .. }
        ) {
            return Err(WorkError::Invalid);
        }
        let binding_request = shell.work_profile_binding();
        let binding = tokio::time::timeout(std::time::Duration::from_secs(8), async move {
            loop {
                if let Some(binding) = binding_request.try_recv() {
                    break binding;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| WorkError::Unavailable)?;
        let zephium_app::AgentWorkProfileReadiness::Ready(binding) = binding else {
            return Err(WorkError::ProfileUnavailable);
        };
        if binding.profile() != profile {
            return Err(WorkError::ProfileUnavailable);
        }
        let search = search_backend(profile, &models).await?;
        record_diagnostic(format_args!(
            "work: phase=lead state=models lead={} page={} light={}",
            models.lead.entry.id, models.page.entry.id, models.light.entry.id
        ));
        let page = page_model(&models)?;
        // Typed decisions the recommended backend leaves open go to the light
        // model; one that cannot call tools leaves them to the page model.
        let light = zephium_agentic::AgentProviderModelClient::try_new(
            &models.light.entry,
            models.light.client.clone(),
        )
        .ok();
        let callback = shell.callback_handle();
        let work = command.work;
        let started = std::time::Instant::now();
        let service = WorkLeadService::new(shell).with_diagnostic(|event| {
            record_diagnostic(format_args!("work: phase=lead event={event:?}"));
        });
        let projection = service
            .run(
                profile,
                command,
                context,
                models,
                search.as_ref(),
                |probe, request| {
                    let callback = &callback;
                    let page = page.clone();
                    let light = light.clone();
                    async move {
                        // The catalog model is unused: the chosen page model carries its own.
                        let mut settings = zephium_work_composition::durable_runtime::WorkBrowserAdapterSettings::new(
                            binding,
                            zephium_agent_controller::AgentBrowserModel::Gpt6Luna,
                            zephium_app::AgentWorkApplicationConfig::new(
                                zephium_agent_runtime::AgentRuntimeConfig::STANDARD,
                                AgentProviderTransportConfig::STANDARD,
                            ),
                            zephium_agent_controller::AgentBrowserModelAccess::Chosen(page),
                        );
                        settings.decisions = super::super::work_decision::composition_preference(
                            super::super::work_decision::selected_choice(profile).await,
                        );
                        settings.decision_model = light;
                        settings.refusal_diagnostic = Some(|stage| {
                            record_diagnostic(format_args!("work: phase=agent_browser refused={stage}"));
                        });
                        #[cfg(feature = "work-development-traces")]
                        let settings = {
                            let mut settings = settings;
                            settings.retain_public_responses = true;
                            settings.diagnostic = Some(|attempt, snapshot| {
                                record_diagnostic(format_args!("work: attempt={attempt} phase=agent_browser state={:?} failure={:?} persistence_failure={:?}", snapshot.phase, snapshot.failure, snapshot.persistence_failure));
                            });
                            settings.model_diagnostic = Some(record_browser_diagnostic);
                            settings.stage_diagnostic = Some(|stage| {
                                record_diagnostic(format_args!("work: phase=agent_browser stage={stage}"));
                            });
                            settings.resource_diagnostic = Some(|cause| {
                                record_diagnostic(format_args!("work: phase=agent_browser resource_failure={cause:?}"));
                            });
                            settings
                        };
                        let result = self.browser.run_agent_step(callback, &probe, request, settings).await;
                        if let Err(error) = &result {
                            record_diagnostic(format_args!("work: attempt={} phase=agent_browser failure={error:?}", probe.attempt()));
                        }
                        result
                    }
                },
                |observer| self.activity.track(observer),
            )
            .await;
        match &projection {
            Ok(projection) => {
                if let Some(fact) = projection.executions.last() {
                    record_diagnostic(format_args!(
                        "work: work={work} phase=lead status={:?} steps={} parts={} artifacts={} elapsed_ms={}",
                        fact.status,
                        fact.steps.len(),
                        fact.parts.len(),
                        fact.artifacts.len(),
                        started.elapsed().as_millis()
                    ));
                }
            }
            Err(error) => record_diagnostic(format_args!(
                "work: work={work} phase=lead failure={error:?} elapsed_ms={}",
                started.elapsed().as_millis()
            )),
        }
        Ok(WorkOperationStateV1::Settled {
            response: WorkResponseV1 {
                version: 1,
                profile: profile.to_string(),
                reply: WorkReplyV1::Projection {
                    projection: Box::new(projection?),
                },
            },
        })
    }
}
