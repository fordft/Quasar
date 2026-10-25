//! Release-excluded live qualification for the production semantic adapter.

#[path = "agentic_accessibility_fill_probe.rs"]
mod accessibility_fill;

#[path = "agentic_responder_fill_probe.rs"]
mod responder_fill;

#[path = "agentic_trusted_edit_probe.rs"]
mod trusted_edit;

#[path = "agentic_owned_surface_probe.rs"]
mod owned_surface;

#[path = "agentic_rendering_probe.rs"]
mod rendering;
pub use rendering::MacosAgenticRenderingProbeReport;
#[cfg(feature = "native-agentic-foreground-probe")]
pub(crate) use rendering::{sample_snapshot as sample_foreground_snapshot, RenderingDocumentState};
#[path = "agentic_rendering_opportunity_probe.rs"]
mod rendering_opportunity;
pub use rendering_opportunity::{MacosAgenticRenderingOpportunityReport, RenderingOpportunity};
#[path = "agentic_rendering_presented_probe.rs"]
mod rendering_presented;
pub use rendering_presented::{
    MacosAgenticPresentedRenderingFailure, MacosAgenticPresentedRenderingReport,
};

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use objc2::{
    rc::{Retained, Weak},
    runtime::AnyObject,
    MainThreadOnly as _,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSDate, NSError, NSPoint, NSRect, NSRunLoop, NSSize, NSString,
};
use objc2_web_kit::{WKBackForwardListItem, WKWebView, WKWebsiteDataStore};
use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use zephium_agentic::{
    encode_semantic_runtime_invocation, AgentAccountAttestationId, AgentAccountScope,
    AgentContextAccountBinding, AgentEffectScope, AgentPlanLeaseBinding, AgentPlanLeaseId,
    AgentPlanNodeAuthority, AgentPlanNodeId, AgentPlanNodeScope, AgentPolicyInstant,
    AgentRunBudget, AgentRunManifest, AgentRunManifestId, AgentRunScope, ContextCapabilities,
    ContextCapability, ContextId, ContextIdentity, ContextKind, ContextNavigationTarget,
    ContextOperationId, ContextOwnedViewport, ContextProfileStorageClass, ContextRegistry,
    ContextRunId, ContextSettlement, FixtureRoute, FixtureServer, FixtureServerError, FrameId,
    SemanticActionExecutionBackend, SemanticActionExecutionInstant, SemanticActionNativeReadiness,
    SemanticActionNativeSettlement, SemanticActionQualificationError, SemanticActionText,
    SemanticClickQualificationExecution, SemanticCompleteness, SemanticEffectClass,
    SemanticExpansionKind, SemanticFillQualificationExecution, SemanticFrameJoin,
    SemanticFrameTrust, SemanticFrameUnsupported, SemanticInvocationId, SemanticNode,
    SemanticObservationAssembler, SemanticObservationBudget, SemanticObservationId,
    SemanticObservationRequest, SemanticOperationClass, SemanticOrigin, SemanticRole,
    SemanticRuntimeBudget, SemanticRuntimeFault, SemanticRuntimeInvocation,
    SemanticRuntimePortFailure, SemanticRuntimeResultError, SemanticSensitivity,
    SemanticSettleInstant, SemanticSnapshot, SemanticSnapshotGeneration, SemanticState,
    SemanticValueSummary,
};
use zephium_core::ids::ProfileId;

use super::{
    AgentNavigationCommit, AgentNavigationTerminal, AgentOwnedView, AgentOwnedViewCallbacks,
    NativeContentPolicy,
};

const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(10);
const PUBLIC_NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const ACTION_SECURITY_SETTLE: Duration = Duration::from_millis(125);
const MUTATION_GATE_TIMEOUT: Duration = Duration::from_secs(5);
const MUTATION_APPLY_SETTLE: Duration = Duration::from_millis(250);
const TEARDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const RUN_LOOP_SLICE: Duration = Duration::from_millis(5);
const MAX_DOCUMENT_LOADING_RETRIES: u16 = 512;
const PAGE_WORLD_FILL_RELAY_PROBE_ENV: &str = "ZEPHIUM_PAGE_WORLD_FILL_RELAY_PROBE";
const PAGE_WORLD_FILL_RELAY_HOSTILE_PROBE_ENV: &str = "ZEPHIUM_PAGE_WORLD_FILL_RELAY_HOSTILE_PROBE";
const MODEL_PROBE_POLICY_NOW_MILLIS: u64 = 10_000;
const MODEL_PROBE_POLICY_EXPIRES_MILLIS: u64 = MODEL_PROBE_POLICY_NOW_MILLIS + 10 * 60 * 1_000;
const MODEL_PROBE_MODEL_TOKEN_BUDGET: u64 = 300_000;
const MODEL_PROBE_COST_BUDGET_MICRO_USD: u64 = 1_000_000;
const PUBLIC_DISCOVERY_PROBE_URL: &str = "https://www.wikipedia.org/";

struct ProbeHostView {
    view: Retained<NSView>,
}

struct PendingPrimaryClick {
    execution: SemanticClickQualificationExecution,
    settlement: SemanticActionNativeSettlement,
    admitted_at: Instant,
}

struct PendingModelAction {
    settlement: SemanticActionNativeSettlement,
    admitted_at: Instant,
    wait: zephium_agentic::SemanticWaitCondition,
    settle_millis: u32,
}

enum PendingInitialClick {
    Fixed(Box<PendingPrimaryClick>),
    Model(Box<PendingModelAction>),
}

struct PendingPrimaryFill {
    execution: SemanticFillQualificationExecution,
    settlement: SemanticActionNativeSettlement,
    admitted_at: Instant,
}

impl HasWindowHandle for ProbeHostView {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let pointer = NonNull::from(&*self.view).cast::<c_void>();
        let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(pointer));
        // SAFETY: the retained content view and its retained window outlive
        // the Wry child created from this borrowed handle.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

#[derive(Default)]
struct CallbackState {
    navigation: RefCell<Option<AgentNavigationTerminal>>,
    renderer_lost: Cell<bool>,
    invariant_failed: Cell<bool>,
    callback_panicked: Cell<bool>,
}

impl CallbackState {
    fn failed(&self) -> bool {
        self.renderer_lost.get() || self.invariant_failed.get() || self.callback_panicked.get()
    }

    fn failure_stage(&self) -> Option<&'static str> {
        if self.renderer_lost.get() {
            Some("renderer_lost")
        } else if self.invariant_failed.get() {
            Some("callback_invariant")
        } else if self.callback_panicked.get() {
            Some("callback_panicked")
        } else {
            None
        }
    }
}

struct NativeStateGuard<'a> {
    app: &'a NSApplication,
    window: &'a NSWindow,
    page: &'a WKWebView,
    window_main: bool,
    first_responder: Option<usize>,
    allow_hidden_responder_change: bool,
    failure: Cell<Option<&'static str>>,
}

impl NativeStateGuard<'_> {
    fn sample(&self) {
        let failure = if self.window.isVisible() {
            Some("window_became_visible")
        } else if self.window.isKeyWindow() {
            Some("window_became_key")
        } else if self.window.isMainWindow() != self.window_main {
            Some("window_main_state_changed")
        } else if !self.page.isHidden() {
            Some("page_became_visible")
        } else if self.app.isActive() {
            Some("application_became_active")
        } else if self
            .window
            .firstResponder()
            .map(|responder| Retained::as_ptr(&responder).addr())
            != self.first_responder
        {
            if self.allow_hidden_responder_change {
                None
            } else {
                Some("hidden_first_responder_changed")
            }
        } else {
            None
        };
        if self.failure.get().is_none() {
            self.failure.set(failure);
        }
    }

    fn failed(&self) -> bool {
        self.failure.get().is_some()
    }

    fn failure_stage(&self) -> Option<&'static str> {
        self.failure.get()
    }
}

enum ProbeNativeState<'native> {
    Hidden(NativeStateGuard<'native>),
    Presented(rendering_presented::PresentedStateGuard<'native>),
}

impl ProbeNativeState<'_> {
    fn sample(&self) {
        match self {
            Self::Hidden(guard) => guard.sample(),
            Self::Presented(guard) => guard.sample(),
        }
    }

    fn failed(&self) -> bool {
        match self {
            Self::Hidden(guard) => guard.failed(),
            Self::Presented(guard) => guard.failed(),
        }
    }

    fn failure_stage(&self) -> Option<&'static str> {
        match self {
            Self::Hidden(guard) => guard.failure_stage(),
            Self::Presented(guard) => guard.failure_stage(),
        }
    }
}

/// The fixture one probe case runs against, with the run's capture counters.
struct ProbeCase<'a> {
    url: &'a str,
    case: &'a str,
    next_invocation: &'a mut u64,
    successful_snapshots: &'a mut u8,
}

struct ProbeRuntime<'a, 'native> {
    callbacks: &'a CallbackState,
    run_loop: &'a NSRunLoop,
    native_guard: &'a ProbeNativeState<'native>,
}

impl ProbeRuntime<'_, '_> {
    fn failed(&self) -> bool {
        self.callbacks.failed() || self.native_guard.failed()
    }

    fn pump(&self) {
        pump_once(self.run_loop, Some(self.native_guard));
        if self.callbacks.failed() {
            return;
        }
        if let ProbeNativeState::Presented(guard) = self.native_guard {
            guard.pump_appkit_event();
        }
    }

    fn failure_stage(&self) -> Option<&'static str> {
        self.callbacks
            .failure_stage()
            .or_else(|| self.native_guard.failure_stage())
    }
}

struct CapturedSnapshot {
    request: SemanticObservationRequest,
    snapshot: SemanticSnapshot,
}

type ModelInitialCallback<'a> = dyn FnMut(
        &zephium_agentic::SemanticObservation,
        MacosAgenticSemanticProbeAuthority,
    ) -> Result<zephium_agentic::SemanticActionNativeRequest, ()>
    + 'a;
type ModelContinuationCallback<'a> = dyn FnMut(
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticActionNativeSettlement,
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticSettleInstant,
    ) -> Result<zephium_agentic::SemanticActionNativeRequest, ()>
    + 'a;
type ModelFinishCallback<'a> = dyn FnMut(
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticActionNativeSettlement,
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticSettleInstant,
    ) -> Result<(), ()>
    + 'a;

type WorkflowInitialCallback<'a> = dyn FnMut(
        &zephium_agentic::SemanticObservation,
        MacosAgenticSemanticProbeAuthority,
        zephium_agentic::ContextAutomationState,
    ) -> Result<zephium_agentic::SemanticActionNativeRequest, ()>
    + 'a;
type WorkflowNextCallback<'a> = dyn FnMut(
        &zephium_agentic::SemanticObservation,
        SemanticActionNativeSettlement,
        &zephium_agentic::SemanticObservation,
        SemanticSettleInstant,
        zephium_agentic::ContextAutomationState,
    ) -> Result<Option<zephium_agentic::SemanticActionNativeRequest>, ()>
    + 'a;

/// Closed page and action sequence for a two-turn model qualification run.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum MacosAgenticSemanticTwoActionScenario {
    /// Fixed loopback click followed by fill.
    FixedClickFill,
    /// Fixed-origin Wikipedia fill followed by language selection.
    PublicFillSelect,
}

enum ProbeMode<'a> {
    Full,
    Rendering(&'a mut Option<MacosAgenticRenderingProbeReport>),
    RenderingPresented(&'a mut Option<rendering_presented::PresentedOutcome>),
    RenderingOpportunity {
        opportunity: RenderingOpportunity,
        report: &'a mut Option<MacosAgenticRenderingOpportunityReport>,
    },
    HistoryRuntime(&'a mut Option<MacosAgenticHistoryRuntimeProbeReport>),
    ModelClick(&'a mut ModelInitialCallback<'a>),
    ModelPublicFill(&'a mut ModelInitialCallback<'a>),
    ModelWorkflow {
        initial: &'a mut WorkflowInitialCallback<'a>,
        next: &'a mut WorkflowNextCallback<'a>,
    },
    ModelTwoAction {
        scenario: MacosAgenticSemanticTwoActionScenario,
        prepare_initial: &'a mut ModelInitialCallback<'a>,
        prepare_continuation: &'a mut ModelContinuationCallback<'a>,
        finish: &'a mut ModelFinishCallback<'a>,
    },
}

/// Release-excluded evidence for exact native history identity and restoration
/// lifecycle. It contains no page text or URL.
#[derive(Debug)]
pub struct MacosAgenticHistoryRuntimeProbeReport {
    /// The immediate predecessor was the exact retained native item.
    pub exact_item_identity: bool,
    /// WebKit restored the original JavaScript document rather than cold-loading it.
    pub bfcache_restored: bool,
    /// A trusted pageshow notification ran after the exact history traversal.
    pub pageshow_observed: bool,
    /// The pageshow event identified a BFCache restoration.
    pub pageshow_persisted: bool,
    /// The parked isolated-world runtime resumed and produced a fresh snapshot.
    pub semantic_runtime_reactivated: bool,
}

/// Fixed run authority bound to one live release-excluded semantic context.
///
/// The bundle is minted beside the context registry from the exact current
/// context join. Callers can consume it once to construct the provider policy
/// turn, but cannot select a profile, run, origin, account, effect scope, or
/// budget independently.
#[doc(hidden)]
#[must_use]
pub struct MacosAgenticSemanticProbeAuthority {
    manifest: AgentRunManifest,
    lease: AgentPlanLeaseBinding,
    account: AgentContextAccountBinding,
    observation: SemanticObservationRequest,
    frame: SemanticFrameJoin,
    invocation: SemanticInvocationId,
    snapshot_generation: SemanticSnapshotGeneration,
    now: AgentPolicyInstant,
}

impl MacosAgenticSemanticProbeAuthority {
    /// Consumes the exact authority needed to construct one pre-observed Terra turn.
    pub fn into_parts(
        self,
    ) -> (
        AgentRunManifest,
        AgentPlanLeaseBinding,
        AgentContextAccountBinding,
        SemanticObservationRequest,
        SemanticFrameJoin,
        SemanticInvocationId,
        SemanticSnapshotGeneration,
        AgentPolicyInstant,
    ) {
        (
            self.manifest,
            self.lease,
            self.account,
            self.observation,
            self.frame,
            self.invocation,
            self.snapshot_generation,
            self.now,
        )
    }
}

/// Move-only terminal returned by a release-excluded model-action session.
///
/// Page content remains confined to the semantic snapshot and this value has
/// no logging implementation. The caller must rejoin it with the exact
/// model-proposal owner that created the native request.
#[doc(hidden)]
#[must_use]
pub struct MacosAgenticSemanticModelActionTerminal {
    settlement: SemanticActionNativeSettlement,
    snapshot: SemanticSnapshot,
    observed_at: SemanticSettleInstant,
}

impl MacosAgenticSemanticModelActionTerminal {
    /// Consumes the terminal into the exact native settlement, fresh snapshot,
    /// and trusted monotonic observation instant required by verification.
    pub fn into_parts(
        self,
    ) -> (
        SemanticActionNativeSettlement,
        SemanticSnapshot,
        SemanticSettleInstant,
    ) {
        (self.settlement, self.snapshot, self.observed_at)
    }
}

/// Compatibility name for the original fixed-fixture click probe.
pub type MacosAgenticSemanticModelClickTerminal = MacosAgenticSemanticModelActionTerminal;

struct PendingTeardown {
    execution: Result<Option<MacosAgenticSemanticModelActionTerminal>, &'static str>,
    page: Weak<WKWebView>,
    window: Weak<NSWindow>,
    store: Weak<WKWebsiteDataStore>,
    fixture_failure: Option<&'static str>,
}

pub(crate) fn run() -> Result<(), &'static str> {
    let pending = objc2::rc::autoreleasepool(|_| begin(ProbeMode::Full))?;
    match finish(pending)? {
        None => Ok(()),
        Some(_) => Err("unexpected_model_terminal"),
    }
}

pub(crate) fn run_history_runtime() -> Result<MacosAgenticHistoryRuntimeProbeReport, &'static str> {
    let mut report = None;
    let pending = objc2::rc::autoreleasepool(|_| begin(ProbeMode::HistoryRuntime(&mut report)))?;
    match finish(pending)? {
        None => report.ok_or("history_probe_report"),
        Some(_) => Err("unexpected_model_terminal"),
    }
}

pub(crate) fn run_rendering() -> Result<MacosAgenticRenderingProbeReport, &'static str> {
    let mut report = None;
    let pending = objc2::rc::autoreleasepool(|_| begin(ProbeMode::Rendering(&mut report)))?;
    if finish(pending)?.is_some() {
        return Err("unexpected_model_terminal");
    }
    // A measurement is returned only after the same original native owners,
    // policy/runtime registrations and loopback worker have drained.
    report.ok_or("rendering_report_missing")
}

pub(crate) fn run_rendering_opportunity(
    opportunity: RenderingOpportunity,
) -> Result<MacosAgenticRenderingOpportunityReport, &'static str> {
    let mut report = None;
    let pending = objc2::rc::autoreleasepool(|_| {
        begin(ProbeMode::RenderingOpportunity {
            opportunity,
            report: &mut report,
        })
    })?;
    if finish(pending)?.is_some() {
        return Err("unexpected_model_terminal");
    }
    report.ok_or("rendering_opportunity_report_missing")
}

pub(crate) fn run_rendering_presented() -> rendering_presented::PresentedOutcome {
    let mut report = None;
    let pending =
        objc2::rc::autoreleasepool(|_| begin(ProbeMode::RenderingPresented(&mut report)))?;
    match finish(pending) {
        Ok(None) => report.ok_or_else(|| {
            MacosAgenticPresentedRenderingFailure::from("rendering_presented_report_missing")
        })?,
        Ok(Some(_)) => Err(MacosAgenticPresentedRenderingFailure::supersede(
            "unexpected_model_terminal",
            report,
        )),
        Err(stage) => Err(MacosAgenticPresentedRenderingFailure::supersede(
            stage, report,
        )),
    }
}

pub(crate) fn run_model_click(
    mut prepare: impl FnMut(
        &zephium_agentic::SemanticObservation,
        MacosAgenticSemanticProbeAuthority,
    ) -> Result<zephium_agentic::SemanticActionNativeRequest, ()>,
) -> Result<MacosAgenticSemanticModelClickTerminal, &'static str> {
    let pending = objc2::rc::autoreleasepool(|_| begin(ProbeMode::ModelClick(&mut prepare)))?;
    finish(pending)?.ok_or("missing_model_terminal")
}

pub(crate) fn run_model_public_fill(
    mut prepare: impl FnMut(
        &zephium_agentic::SemanticObservation,
        MacosAgenticSemanticProbeAuthority,
    ) -> Result<zephium_agentic::SemanticActionNativeRequest, ()>,
) -> Result<MacosAgenticSemanticModelActionTerminal, &'static str> {
    let pending = objc2::rc::autoreleasepool(|_| begin(ProbeMode::ModelPublicFill(&mut prepare)))?;
    finish(pending)?.ok_or("missing_model_terminal")
}

pub(crate) fn run_model_two_action(
    scenario: MacosAgenticSemanticTwoActionScenario,
    mut prepare_initial: impl FnMut(
        &zephium_agentic::SemanticObservation,
        MacosAgenticSemanticProbeAuthority,
    ) -> Result<zephium_agentic::SemanticActionNativeRequest, ()>,
    mut prepare_continuation: impl FnMut(
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticActionNativeSettlement,
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticSettleInstant,
    )
        -> Result<zephium_agentic::SemanticActionNativeRequest, ()>,
    mut verify_final: impl FnMut(
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticActionNativeSettlement,
        &zephium_agentic::SemanticObservation,
        zephium_agentic::SemanticSettleInstant,
    ) -> Result<(), ()>,
) -> Result<(), &'static str> {
    let pending = objc2::rc::autoreleasepool(|_| {
        begin(ProbeMode::ModelTwoAction {
            scenario,
            prepare_initial: &mut prepare_initial,
            prepare_continuation: &mut prepare_continuation,
            finish: &mut verify_final,
        })
    })?;
    match finish(pending)? {
        None => Ok(()),
        Some(_) => Err("unexpected_model_terminal"),
    }
}

pub(crate) fn run_model_workflow(
    mut initial: impl FnMut(
        &zephium_agentic::SemanticObservation,
        MacosAgenticSemanticProbeAuthority,
        zephium_agentic::ContextAutomationState,
    ) -> Result<zephium_agentic::SemanticActionNativeRequest, ()>,
    mut next: impl FnMut(
        &zephium_agentic::SemanticObservation,
        SemanticActionNativeSettlement,
        &zephium_agentic::SemanticObservation,
        SemanticSettleInstant,
        zephium_agentic::ContextAutomationState,
    ) -> Result<Option<zephium_agentic::SemanticActionNativeRequest>, ()>,
) -> Result<(), &'static str> {
    let pending = objc2::rc::autoreleasepool(|_| {
        begin(ProbeMode::ModelWorkflow {
            initial: &mut initial,
            next: &mut next,
        })
    })?;
    match finish(pending)? {
        None => Ok(()),
        Some(_) => Err("unexpected_model_terminal"),
    }
}

fn begin(mut mode: ProbeMode<'_>) -> Result<PendingTeardown, &'static str> {
    let full_probe = matches!(&mode, ProbeMode::Full);
    let rendering_probe = matches!(
        &mode,
        ProbeMode::Rendering(_)
            | ProbeMode::RenderingOpportunity { .. }
            | ProbeMode::RenderingPresented(_)
            | ProbeMode::HistoryRuntime(_)
    );
    let public_fill_probe = matches!(
        &mode,
        ProbeMode::ModelPublicFill(_)
            | ProbeMode::ModelWorkflow { .. }
            | ProbeMode::ModelTwoAction {
                scenario: MacosAgenticSemanticTwoActionScenario::PublicFillSelect,
                ..
            }
    );
    let page_relay_probe = full_probe && page_world_fill_relay_probe_enabled();
    let ax_fill_probe = full_probe
        && std::env::var_os(accessibility_fill::ENV).as_deref() == Some(std::ffi::OsStr::new("1"));
    let responder_case = if full_probe {
        responder_fill::case_from_env()?
    } else {
        None
    };
    let trusted_case = if full_probe {
        trusted_edit::case_from_env()?
    } else {
        None
    };
    let surface_case = if full_probe {
        owned_surface::case_from_env()?
    } else {
        None
    };
    if surface_case.is_some()
        && (trusted_case.is_some() || responder_case.is_some() || ax_fill_probe || page_relay_probe)
    {
        return Err("surface_conflicting_probe");
    }
    if (responder_case.is_some() && (ax_fill_probe || page_relay_probe))
        || (trusted_case.is_some()
            && (ax_fill_probe || page_relay_probe || responder_case.is_some()))
    {
        return Err("responder_conflicting_probe");
    }
    let hostile_relay_probe = full_probe && page_world_fill_relay_hostile_probe_enabled();
    if hostile_relay_probe && !page_relay_probe {
        return Err("relay_hostile_requires_page_relay");
    }
    let mtm = MainThreadMarker::new().ok_or("main_thread")?;
    let server = FixtureServer::start().map_err(|_| "fixture_start")?;
    let profile = ProfileId::generate();
    let store = super::new_ephemeral_data_store().map_err(|_| "profile_construct")?;

    let app = NSApplication::sharedApplication(mtm);
    if app.isActive() {
        return Err("focus_baseline");
    }
    if matches!(&mode, ProbeMode::RenderingPresented(_))
        || ax_fill_probe
        || responder_case.is_some()
        || trusted_case.is_some()
        || surface_case.is_some()
    {
        rendering_presented::initialize_inactive(&app)?;
    } else {
        if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
            return Err("application_policy");
        }
        app.finishLaunching();
    }

    let window = new_window(mtm)?;
    let content = window.contentView().ok_or("window_construct")?;
    let nested = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(37.0, 29.0), NSSize::new(680.0, 560.0)),
    );
    content.addSubview(&nested);
    let host = ProbeHostView { view: nested };
    let callbacks = Rc::new(CallbackState::default());
    let navigation_callbacks = Rc::clone(&callbacks);
    let renderer_callbacks = Rc::clone(&callbacks);
    let invariant_callbacks = Rc::clone(&callbacks);
    let panic_callbacks = Rc::clone(&callbacks);

    let mut registry = ContextRegistry::new();
    let identity = ContextIdentity::new(
        ContextId::generate(),
        ContextRunId::generate(),
        profile,
        ContextKind::Owned,
    );
    let capabilities = ContextCapabilities::try_new(
        ContextKind::Owned,
        if rendering_probe {
            &[ContextCapability::Observe, ContextCapability::Navigate]
        } else {
            &[
                ContextCapability::Observe,
                ContextCapability::Navigate,
                ContextCapability::Act,
            ]
        },
    )
    .map_err(|_| "context_construct")?;
    registry
        .reserve(identity, capabilities)
        .map_err(|_| "context_construct")?;
    let construction = registry
        .begin_context(
            identity.id(),
            ContextOperationId::new(1).ok_or("context_construct")?,
        )
        .map_err(|_| "context_construct")?;

    let mut view = super::build_owned_agent_view(
        &host,
        ContextOwnedViewport::STANDARD,
        profile,
        ContextProfileStorageClass::Ephemeral,
        Some(&store),
        AgentOwnedViewCallbacks::new(
            move |terminal| {
                if navigation_callbacks
                    .navigation
                    .try_borrow_mut()
                    .is_ok_and(|mut slot| {
                        if slot.is_some() {
                            false
                        } else {
                            *slot = Some(terminal);
                            true
                        }
                    })
                {
                    return;
                }
                navigation_callbacks.invariant_failed.set(true);
            },
            || {},
            move || renderer_callbacks.renderer_lost.set(true),
            move || invariant_callbacks.invariant_failed.set(true),
            move || panic_callbacks.callback_panicked.set(true),
        ),
    )
    .map_err(|_| "view_construct")?;
    let policy = super::install_content_policy_on_view(view.view(), &NativeContentPolicy::AllowAll)
        .map_err(|_| "content_policy")?;
    registry
        .settle_construction(identity.id(), construction, ContextSettlement::Applied)
        .map_err(|_| "context_construct")?;

    let page = super::native_webview(view.view());
    let native_guard = ProbeNativeState::Hidden(NativeStateGuard {
        app: &app,
        window: &window,
        page: &page,
        window_main: window.isMainWindow(),
        first_responder: window
            .firstResponder()
            .map(|responder| Retained::as_ptr(&responder).addr()),
        // A public page may autofocus within its own hidden, non-key window.
        // That internal responder state is not user focus theft; visibility,
        // key-window, main-window, and application activation remain strict.
        allow_hidden_responder_change: public_fill_probe || ax_fill_probe,
        failure: Cell::new(None),
    });
    native_guard.sample();
    let run_loop = NSRunLoop::mainRunLoop();
    let runtime = ProbeRuntime {
        callbacks: &callbacks,
        run_loop: &run_loop,
        native_guard: &native_guard,
    };
    let mut next_invocation = 1_u64;
    let mut successful_snapshots = 0_u8;
    let execution = (|| {
        if let ProbeMode::HistoryRuntime(report) = &mut mode {
            let version = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
            if version.majorVersion < 26 {
                return Err("history_probe_requires_macos_26");
            }
            let first_url = server.url(FixtureRoute::SemanticRuntime);
            let second_url = server.url(FixtureRoute::SemanticRuntimeReplacement);
            let (_first_context, _) = navigate_with_receipt(
                &mut view,
                &mut registry,
                identity.id(),
                2,
                &first_url,
                &runtime,
            )?;
            view.enroll_current_work_history_get(
                ContextNavigationTarget::parse(&first_url)
                    .map_err(|_| "history_probe_first_target")?,
            )
            .map_err(|_| "history_probe_first_enrollment")?;
            install_history_lifecycle_witness(&page, &runtime)?;
            let first_item = native_current_history_item(&page)?;
            if native_item_url(&first_item).as_deref() != Some(first_url.as_str()) {
                return Err("history_probe_first_item");
            }
            park_semantic_runtime(&mut view, &runtime)?;
            let (_second_context, _) = navigate_with_receipt(
                &mut view,
                &mut registry,
                identity.id(),
                3,
                &second_url,
                &runtime,
            )?;
            view.enroll_current_work_history_get(
                ContextNavigationTarget::parse(&second_url)
                    .map_err(|_| "history_probe_second_target")?,
            )
            .map_err(|_| "history_probe_second_enrollment")?;
            let list = unsafe { page.backForwardList() };
            let current = unsafe { list.currentItem() }.ok_or("history_probe_current_item")?;
            let predecessor = unsafe { list.backItem() }.ok_or("history_probe_back_item")?;
            let exact_item_identity = Retained::as_ptr(&predecessor)
                == Retained::as_ptr(&first_item)
                && native_item_url(&current).as_deref() == Some(second_url.as_str());
            if !exact_item_identity {
                return Err("history_probe_item_identity");
            }
            let restored_context = history_back_with_receipt(
                &mut view,
                &mut registry,
                identity.id(),
                4,
                &first_url,
                &first_item,
                &runtime,
            )?;
            let restored = native_current_history_item(&page)?;
            if Retained::as_ptr(&restored) != Retained::as_ptr(&first_item) {
                return Err("history_probe_restored_item");
            }
            let lifecycle = read_history_lifecycle_witness(&page, &runtime)?;
            let restored_capture = capture_snapshot(
                &view,
                restored_context,
                &first_url,
                SemanticSnapshotGeneration::INITIAL,
                &mut next_invocation,
                &mut successful_snapshots,
                &runtime,
            )?;
            let semantic_runtime_reactivated = restored_capture.snapshot.frame().context()
                == restored_context
                && restored_capture.snapshot.generation() == SemanticSnapshotGeneration::INITIAL;
            **report = Some(MacosAgenticHistoryRuntimeProbeReport {
                exact_item_identity,
                bfcache_restored: lifecycle[0],
                pageshow_observed: lifecycle[1],
                pageshow_persisted: lifecycle[2],
                semantic_runtime_reactivated,
            });
            return Ok(None);
        }
        if let ProbeMode::RenderingPresented(report) = &mut mode {
            let url = server.url(FixtureRoute::SemanticRendering);
            let (context, operation) =
                navigate_with_receipt(&mut view, &mut registry, identity.id(), 2, &url, &runtime)?;
            let outcome =
                rendering_presented::measure(&view, context, operation, &url, &runtime, &host.view);
            let refusal = outcome.as_ref().err().map(|failure| failure.stage);
            **report = Some(outcome);
            return refusal.map_or(Ok(None), Err);
        }
        if let ProbeMode::RenderingOpportunity {
            opportunity,
            report,
        } = &mut mode
        {
            let url = server.url(FixtureRoute::SemanticRendering);
            let (context, operation) =
                navigate_with_receipt(&mut view, &mut registry, identity.id(), 2, &url, &runtime)?;
            **report = Some(rendering_opportunity::measure(
                &view,
                context,
                operation,
                &url,
                &runtime,
                *opportunity,
            )?);
            return Ok(None);
        }
        if let ProbeMode::Rendering(report) = &mut mode {
            let url = server.url(FixtureRoute::SemanticRendering);
            let (context, operation) =
                navigate_with_receipt(&mut view, &mut registry, identity.id(), 2, &url, &runtime)?;
            **report = Some(rendering::measure(
                &view, context, operation, &url, &runtime,
            )?);
            return Ok(None);
        }
        let mut first_url = if public_fill_probe {
            PUBLIC_DISCOVERY_PROBE_URL.to_owned()
        } else {
            server.url(if page_relay_probe {
                if hostile_relay_probe {
                    FixtureRoute::SemanticRuntimeRelayHostile
                } else {
                    FixtureRoute::SemanticRuntimeRelay
                }
            } else {
                FixtureRoute::SemanticRuntime
            })
        };
        if ax_fill_probe {
            first_url.push_str("#native-ax-fill");
        }
        if let Some(case) = surface_case {
            // localhost is a valid local WebAuthn RP host; a numeric IP would
            // reject before reaching its native capability path.
            first_url = server
                .url(FixtureRoute::OwnedSurfaceProbe)
                .replace("127.0.0.1", "localhost");
            first_url.push('#');
            first_url.push_str(case);
        }
        if let Some(case) = responder_case {
            first_url.push_str("#native-responder-");
            first_url.push_str(case);
        }
        if let Some(case) = trusted_case {
            first_url.push_str("#native-unit-");
            first_url.push_str(case);
        }
        let first = navigate(
            &mut view,
            &mut registry,
            identity.id(),
            2,
            &first_url,
            &runtime,
        )?;
        let first_capture = capture_snapshot(
            &view,
            first,
            &first_url,
            SemanticSnapshotGeneration::INITIAL,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        if let Some(case) = surface_case {
            view.attest(profile, ContextProfileStorageClass::Ephemeral, Some(&store))
                .map_err(|_| "surface_hidden_attestation")?;
            rendering_presented::with_ax_fixture(&runtime, &host.view, |presented| {
                owned_surface::run(
                    &view,
                    &window,
                    &store,
                    first_capture,
                    ProbeCase {
                        url: &first_url,
                        case,
                        next_invocation: &mut next_invocation,
                        successful_snapshots: &mut successful_snapshots,
                    },
                    presented,
                )
            })?;
            return Ok(None);
        }
        if let Some(case) = trusted_case {
            view.attest(profile, ContextProfileStorageClass::Ephemeral, Some(&store))
                .map_err(|_| "trusted_hidden_attestation")?;
            rendering_presented::with_ax_fixture(&runtime, &host.view, |presented| {
                trusted_edit::run(
                    &view,
                    &window,
                    &store,
                    first_capture,
                    ProbeCase {
                        url: &first_url,
                        case,
                        next_invocation: &mut next_invocation,
                        successful_snapshots: &mut successful_snapshots,
                    },
                    presented,
                )
            })?;
            return Ok(None);
        }
        if let Some(case) = responder_case {
            rendering_presented::with_ax_fixture(&runtime, &host.view, |presented| {
                responder_fill::run(
                    &view,
                    &window,
                    first_capture,
                    ProbeCase {
                        url: &first_url,
                        case,
                        next_invocation: &mut next_invocation,
                        successful_snapshots: &mut successful_snapshots,
                    },
                    presented,
                )
            })?;
            return Ok(None);
        }
        if ax_fill_probe {
            rendering_presented::with_ax_fixture(&runtime, &host.view, |presented| {
                accessibility_fill::run(
                    &view,
                    &window,
                    first_capture,
                    &first_url,
                    presented,
                    &mut next_invocation,
                    &mut successful_snapshots,
                )
            })?;
            return Ok(None);
        }
        if public_fill_probe {
            verify_public_discovery_snapshot(&first_capture.snapshot)?;
        } else {
            verify_first_snapshot(&first_capture.snapshot)?;
        }
        let page_origin_blocked = !hostile_relay_probe
            || snapshot_contains(&first_capture.snapshot, "Semantic page-origin fill blocked");
        if hostile_relay_probe
            && !snapshot_contains(
                &first_capture.snapshot,
                "Semantic relay transport legacy command-gone reproduced",
            )
        {
            return Err("relay_batching_filter_or_overflow");
        }
        let first_generation = first_capture.snapshot.generation();
        let first_observation = assemble_observation(first_capture)?;
        if let ProbeMode::ModelWorkflow { initial, next } = &mut mode {
            let authority = model_probe_authority_with_budget(&first_observation, 16)?;
            registry
                .acknowledge_observation(identity.id(), first)
                .map_err(|_| "workflow_observation_ack")?;
            let automation = registry
                .automation_state(identity.id())
                .map_err(|_| "workflow_context")?;
            let mut request = initial(&first_observation, authority, automation)
                .map_err(|()| "workflow_initial")?;
            let mut baseline = first_observation;
            let mut generation = first_generation;
            for _ in 0..8 {
                if !matches!(
                    request.kind(),
                    zephium_agentic::SemanticActionKind::Fill
                        | zephium_agentic::SemanticActionKind::Select
                ) {
                    return Err("workflow_action_kind");
                }
                let pending = execute_model_action(&view, request, None, &runtime)?;
                wait_for_action_security_settle(
                    &runtime,
                    model_action_settle_delay(pending.wait, pending.settle_millis)?,
                )?;
                generation = generation.next().ok_or("workflow_generation")?;
                let capture = capture_snapshot(
                    &view,
                    first,
                    &first_url,
                    generation,
                    &mut next_invocation,
                    &mut successful_snapshots,
                    &runtime,
                )
                .map_err(|reason| match reason {
                    "snapshot_timeout" => "workflow_snapshot_timeout",
                    "snapshot_result" => "workflow_snapshot_result",
                    reason => reason,
                })?;
                generation = capture.snapshot.generation();
                let current = assemble_observation(capture)?;
                let observed_at = action_observed_at(pending.admitted_at)?;
                registry
                    .acknowledge_observation(identity.id(), first)
                    .map_err(|_| "workflow_observation_ack")?;
                let automation = registry
                    .automation_state(identity.id())
                    .map_err(|_| "workflow_context")?;
                match next(
                    &baseline,
                    pending.settlement,
                    &current,
                    observed_at,
                    automation,
                )
                .map_err(|()| "workflow_next")?
                {
                    None => return Ok(None),
                    Some(next_request) => request = next_request,
                }
                baseline = current;
            }
            return Err("workflow_action_limit");
        }
        let pending_click = match &mut mode {
            ProbeMode::ModelWorkflow { .. }
            | ProbeMode::Rendering(_)
            | ProbeMode::RenderingPresented(_)
            | ProbeMode::RenderingOpportunity { .. }
            | ProbeMode::HistoryRuntime(_) => return Err("workflow_state"),
            ProbeMode::Full => PendingInitialClick::Fixed(Box::new(execute_primary_click(
                &view,
                &first_observation,
                &runtime,
            )?)),
            ProbeMode::ModelClick(prepare) => {
                let authority = model_probe_authority(&first_observation)?;
                let request =
                    prepare(&first_observation, authority).map_err(|()| "model_action_prepare")?;
                PendingInitialClick::Model(Box::new(execute_model_action(
                    &view,
                    request,
                    Some(zephium_agentic::SemanticActionKind::Click),
                    &runtime,
                )?))
            }
            ProbeMode::ModelPublicFill(prepare) => {
                let authority = model_probe_authority(&first_observation)?;
                let request =
                    prepare(&first_observation, authority).map_err(|()| "model_action_prepare")?;
                PendingInitialClick::Model(Box::new(execute_model_action(
                    &view,
                    request,
                    Some(zephium_agentic::SemanticActionKind::Fill),
                    &runtime,
                )?))
            }
            ProbeMode::ModelTwoAction {
                scenario,
                prepare_initial,
                ..
            } => {
                let authority = model_probe_authority(&first_observation)?;
                let request = prepare_initial(&first_observation, authority)
                    .map_err(|()| "model_action_prepare")?;
                let expected_kind = match scenario {
                    MacosAgenticSemanticTwoActionScenario::FixedClickFill => {
                        zephium_agentic::SemanticActionKind::Click
                    }
                    MacosAgenticSemanticTwoActionScenario::PublicFillSelect => {
                        zephium_agentic::SemanticActionKind::Fill
                    }
                };
                PendingInitialClick::Model(Box::new(execute_model_action(
                    &view,
                    request,
                    Some(expected_kind),
                    &runtime,
                )?))
            }
        };
        let initial_settle_delay = match &pending_click {
            PendingInitialClick::Fixed(_) => ACTION_SECURITY_SETTLE,
            PendingInitialClick::Model(pending) => {
                model_action_settle_delay(pending.wait, pending.settle_millis)?
            }
        };
        wait_for_action_security_settle(&runtime, initial_settle_delay)?;
        let after_click = capture_snapshot(
            &view,
            first,
            &first_url,
            first_generation.next().ok_or("action_identity")?,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        match pending_click {
            PendingInitialClick::Fixed(pending) => {
                verify_primary_click_execution(*pending, &after_click.snapshot)?;
                verify_primary_click(&after_click.snapshot)?;
            }
            PendingInitialClick::Model(pending) => {
                let observed_at = action_observed_at(pending.admitted_at)?;
                match &mut mode {
                    ProbeMode::ModelClick(_) => {
                        verify_primary_click(&after_click.snapshot)?;
                        return Ok(Some(MacosAgenticSemanticModelActionTerminal {
                            settlement: pending.settlement,
                            snapshot: after_click.snapshot,
                            observed_at,
                        }));
                    }
                    ProbeMode::ModelPublicFill(_) => {
                        return Ok(Some(MacosAgenticSemanticModelActionTerminal {
                            settlement: pending.settlement,
                            snapshot: after_click.snapshot,
                            observed_at,
                        }));
                    }
                    ProbeMode::ModelTwoAction {
                        scenario,
                        prepare_continuation,
                        finish,
                        ..
                    } => {
                        let current_generation = after_click.snapshot.generation();
                        let current_observation = assemble_observation(after_click)?;
                        let request = prepare_continuation(
                            &first_observation,
                            pending.settlement,
                            &current_observation,
                            observed_at,
                        )
                        .map_err(|()| "model_continuation_prepare")?;
                        let expected_kind = match scenario {
                            MacosAgenticSemanticTwoActionScenario::FixedClickFill => {
                                zephium_agentic::SemanticActionKind::Fill
                            }
                            MacosAgenticSemanticTwoActionScenario::PublicFillSelect => {
                                zephium_agentic::SemanticActionKind::Select
                            }
                        };
                        let pending =
                            execute_model_action(&view, request, Some(expected_kind), &runtime)?;
                        wait_for_action_security_settle(
                            &runtime,
                            model_action_settle_delay(pending.wait, pending.settle_millis)?,
                        )?;
                        let after_second = capture_snapshot(
                            &view,
                            first,
                            &first_url,
                            current_generation.next().ok_or("model_action_identity")?,
                            &mut next_invocation,
                            &mut successful_snapshots,
                            &runtime,
                        )?;
                        let final_observed_at = action_observed_at(pending.admitted_at)?;
                        let final_observation = assemble_observation(after_second)?;
                        finish(
                            &current_observation,
                            pending.settlement,
                            &final_observation,
                            final_observed_at,
                        )
                        .map_err(|()| "model_final_verify")?;
                        return Ok(None);
                    }
                    ProbeMode::Full
                    | ProbeMode::ModelWorkflow { .. }
                    | ProbeMode::Rendering(_)
                    | ProbeMode::RenderingPresented(_)
                    | ProbeMode::RenderingOpportunity { .. }
                    | ProbeMode::HistoryRuntime(_) => return Err("model_mode_state"),
                }
            }
        }

        let mut prior_capture = after_click;
        for (name, value, role, batch, attempt, completed) in [
            (
                "Semantic fill text",
                "Quasar fixed text",
                SemanticRole::Textbox,
                2,
                2,
                1,
            ),
            (
                "Semantic fill search",
                "Quasar fixed search",
                SemanticRole::Searchbox,
                3,
                3,
                2,
            ),
            (
                "Semantic fill textarea",
                "  Quasar  fixed textarea\nline two  ",
                SemanticRole::Textbox,
                4,
                4,
                3,
            ),
        ] {
            let prior_generation = prior_capture.snapshot.generation();
            let observation = assemble_observation(prior_capture)?;
            let pending_fill =
                execute_primary_fill(&view, &observation, &runtime, name, value, batch, attempt)?;
            wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
            let after_fill = capture_snapshot(
                &view,
                first,
                &first_url,
                prior_generation.next().ok_or("action_identity")?,
                &mut next_invocation,
                &mut successful_snapshots,
                &runtime,
            )?;
            verify_primary_fill_execution(pending_fill, &after_fill.snapshot)?;
            verify_primary_fill(&after_fill.snapshot, name, value, role, completed)?;
            prior_capture = after_fill;
        }
        if hostile_relay_probe {
            let prior_generation = prior_capture.snapshot.generation();
            let observation = assemble_observation(prior_capture)?;
            let hostile = execute_primary_fill(
                &view,
                &observation,
                &runtime,
                "Semantic hostile fill",
                "Quasar hostile attempt",
                5,
                5,
            )?;
            wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
            let after_hostile = capture_snapshot(
                &view,
                first,
                &first_url,
                prior_generation.next().ok_or("action_identity")?,
                &mut next_invocation,
                &mut successful_snapshots,
                &runtime,
            )?;
            verify_hostile_fill_refusal(hostile, &after_hostile.snapshot)?;

            let recovery_generation = after_hostile.snapshot.generation();
            let observation = assemble_observation(after_hostile)?;
            let recovery = execute_primary_fill(
                &view,
                &observation,
                &runtime,
                "Semantic hostile recovery",
                "Quasar hostile recovery",
                6,
                6,
            )?;
            wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
            let after_recovery = capture_snapshot(
                &view,
                first,
                &first_url,
                recovery_generation.next().ok_or("action_identity")?,
                &mut next_invocation,
                &mut successful_snapshots,
                &runtime,
            )?;
            verify_primary_fill_execution(recovery, &after_recovery.snapshot)?;
            verify_primary_fill(
                &after_recovery.snapshot,
                "Semantic hostile recovery",
                "Quasar hostile recovery",
                SemanticRole::Textbox,
                4,
            )?;
            verify_hostile_fill_recovery(&after_recovery.snapshot)?;

            let credential_generation = after_recovery.snapshot.generation();
            let observation = assemble_observation(after_recovery)?;
            let credential = execute_primary_fill(
                &view,
                &observation,
                &runtime,
                "Semantic hostile credential fill",
                "Quasar credential blocked",
                7,
                7,
            )?;
            wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
            let after_credential = capture_snapshot(
                &view,
                first,
                &first_url,
                credential_generation.next().ok_or("action_identity")?,
                &mut next_invocation,
                &mut successful_snapshots,
                &runtime,
            )?;
            verify_hostile_credential_refusal(credential, &after_credential.snapshot)?;

            let credential_recovery_generation = after_credential.snapshot.generation();
            let observation = assemble_observation(after_credential)?;
            let credential_recovery = execute_primary_fill(
                &view,
                &observation,
                &runtime,
                "Semantic hostile credential fill",
                "Quasar credential recovery",
                8,
                8,
            )?;
            wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
            let after_credential_recovery = capture_snapshot(
                &view,
                first,
                &first_url,
                credential_recovery_generation
                    .next()
                    .ok_or("action_identity")?,
                &mut next_invocation,
                &mut successful_snapshots,
                &runtime,
            )?;
            verify_primary_fill_execution(
                credential_recovery,
                &after_credential_recovery.snapshot,
            )?;
            verify_hostile_credential_recovery(&after_credential_recovery.snapshot)?;
            prior_capture = after_credential_recovery;
        }
        let editable_generation = prior_capture.snapshot.generation();
        let editable_observation = assemble_observation(prior_capture)?;
        let editable_fill = execute_primary_fill(
            &view,
            &editable_observation,
            &runtime,
            "Semantic fill editable",
            "  Quasar fixed editable\nline two  ",
            10,
            10,
        )?;
        wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
        let after_editable = capture_snapshot(
            &view,
            first,
            &first_url,
            editable_generation.next().ok_or("action_identity")?,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        verify_primary_fill_execution(editable_fill, &after_editable.snapshot)?;
        if hostile_relay_probe
            && !snapshot_contains(
                &after_editable.snapshot,
                "Semantic captured-payload retarget blocked",
            )
        {
            return Err("captured_payload_retarget_unauthorized");
        }
        if !page_origin_blocked {
            return Err("page_origin_fill_unauthorized");
        }
        if hostile_relay_probe
            && !snapshot_contains(
                &after_editable.snapshot,
                "Semantic relay attribute interference avoided",
            )
        {
            return Err("relay_editable_transport_interference");
        }
        verify_primary_fill(
            &after_editable.snapshot,
            "Semantic fill editable",
            "  Quasar fixed editable\nline two  ",
            SemanticRole::Textbox,
            if hostile_relay_probe { 5 } else { 4 },
        )?;
        let mut nested_capture = after_editable;
        for (index, desired) in [
            "nested replacement",
            "nested move",
            "nested relabel",
            "nested protected",
            "nested credential",
            "nested editability",
            "nested rich",
        ]
        .into_iter()
        .enumerate()
        {
            let generation = nested_capture.snapshot.generation();
            let observation = assemble_observation(nested_capture)?;
            let pending = execute_primary_fill(
                &view,
                &observation,
                &runtime,
                "Fill support editable ancestor",
                desired,
                20 + index as u64,
                20 + index as u64,
            )?;
            wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
            nested_capture = capture_snapshot(
                &view,
                first,
                &first_url,
                generation.next().ok_or("action_identity")?,
                &mut next_invocation,
                &mut successful_snapshots,
                &runtime,
            )?;
            if index == 0 {
                verify_primary_fill_execution(pending, &nested_capture.snapshot)?;
                if !snapshot_contains(
                    &nested_capture.snapshot,
                    "Nested editor delegated model retained",
                ) {
                    return Err("nested_editor_model_retention");
                }
            } else {
                if pending.settlement.qualification_failure()
                    != Some(zephium_agentic::SemanticActionNativeFailure::AppliedUnverified)
                {
                    return Err("nested_editor_refusal_not_indeterminate");
                }
                let elapsed = u64::try_from(pending.admitted_at.elapsed().as_millis())
                    .map_err(|_| "nested_editor_clock")?;
                let observed_at = SemanticSettleInstant::from_millis(10_000 + elapsed);
                if !matches!(
                    pending.execution.settle_and_verify(
                        pending.settlement,
                        &nested_capture.snapshot,
                        observed_at
                    ),
                    Err(SemanticActionQualificationError::Settlement)
                ) {
                    return Err("nested_editor_retryable_terminal");
                }
                if !snapshot_contains(
                    &nested_capture.snapshot,
                    &format!("Nested editor refused {desired} intact"),
                ) {
                    return Err("nested_editor_context_security");
                }
            }
            if !snapshot_value_is(
                &nested_capture.snapshot,
                "Fill support editable ancestor",
                "nested replacement",
            ) {
                return Err("nested_editor_value");
            }
        }
        drop(nested_capture);
        registry
            .acknowledge_observation(identity.id(), first)
            .map_err(|_| "first_observation")?;

        let mutation_url = server.url(FixtureRoute::SemanticRuntimeMutation);
        let mutation = navigate(
            &mut view,
            &mut registry,
            identity.id(),
            3,
            &mutation_url,
            &runtime,
        )?;
        let mutation_capture = capture_snapshot(
            &view,
            mutation,
            &mutation_url,
            SemanticSnapshotGeneration::INITIAL,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        verify_mutation_before(&mutation_capture.snapshot)?;
        registry
            .acknowledge_observation(identity.id(), mutation)
            .map_err(|_| "mutation_observation")?;

        let mutation_frame = mutation_capture.snapshot.frame().clone();
        let mutation_generation = mutation_capture.snapshot.generation();
        let transient = mutation_capture
            .snapshot
            .nodes()
            .iter()
            .find(|node| node_name_is(node, "Transient mutation anchor"))
            .map(|node| node.reference())
            .ok_or("mutation_anchor_missing")?;
        let frame_boundaries = mutation_capture
            .snapshot
            .nodes()
            .iter()
            .filter(|node| node.role() == SemanticRole::FrameBoundary)
            .map(|node| node.reference())
            .collect::<Vec<_>>();
        let mut assembler =
            SemanticObservationAssembler::new(mutation_capture.request, mutation_capture.snapshot)
                .map_err(|_| "mutation_assembly")?;
        for boundary in frame_boundaries {
            assembler
                .mark_frame_unsupported(
                    FrameId::MAIN,
                    boundary,
                    SemanticFrameUnsupported::PlatformIsolationUnavailable,
                )
                .map_err(|_| "mutation_assembly")?;
        }
        let observation = assembler.finish().map_err(|_| "mutation_assembly")?;
        let expansion_identity = take_semantic_identity(&mut next_invocation)?;
        let expansion = observation
            .begin_expansion(
                SemanticObservationId::new(expansion_identity).ok_or("mutation_identity")?,
                transient,
                &mutation_frame,
                SemanticExpansionKind::Subtree,
                SemanticObservationBudget::INITIAL_FILTERED,
            )
            .map_err(|_| "mutation_expansion")?;
        let expansion_generation = mutation_generation.next().ok_or("mutation_identity")?;
        let expansion_invocation = encode_semantic_runtime_invocation(
            &expansion,
            mutation_frame.clone(),
            SemanticInvocationId::new(expansion_identity).ok_or("mutation_identity")?,
            expansion_generation,
            SemanticRuntimeBudget::INITIAL_FILTERED,
        )
        .map_err(|_| "mutation_encode")?;
        wait_for_mutation_gate(&server, &runtime)?;
        if !server.release_semantic_mutation() {
            return Err("mutation_release");
        }
        wait_for_mutation_application(&server, &runtime)?;
        let expansion_deadline = Instant::now()
            .checked_add(SNAPSHOT_TIMEOUT)
            .ok_or("mutation_timeout")?;
        let expansion_outcome =
            dispatch_invocation(&view, expansion_invocation, &runtime, expansion_deadline)?;
        if !matches!(
            expansion_outcome,
            Err(SemanticRuntimePortFailure::Result(
                SemanticRuntimeResultError::Runtime(SemanticRuntimeFault::AnchorMissing)
            ))
        ) {
            return Err("mutation_stale_anchor");
        }
        let post_mutation_generation = expansion_generation.next().ok_or("mutation_identity")?;
        let post_mutation = capture_snapshot(
            &view,
            mutation,
            &mutation_url,
            post_mutation_generation,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        verify_mutation_after(&post_mutation.snapshot)?;
        registry
            .acknowledge_observation(identity.id(), mutation)
            .map_err(|_| "post_mutation_observation")?;

        let replacement_url = server.url(FixtureRoute::SemanticRuntimeReplacement);
        let replacement = navigate(
            &mut view,
            &mut registry,
            identity.id(),
            4,
            &replacement_url,
            &runtime,
        )?;
        let replacement_capture = capture_snapshot(
            &view,
            replacement,
            &replacement_url,
            SemanticSnapshotGeneration::INITIAL,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        verify_replacement_snapshot(&replacement_capture.snapshot)?;
        registry
            .acknowledge_observation(identity.id(), replacement)
            .map_err(|_| "replacement_observation")?;

        // A fourth native navigation epoch must reinstall and re-attest both
        // immutable worlds before Fill can execute in the replacement epoch.
        let rotated = navigate(
            &mut view,
            &mut registry,
            identity.id(),
            5,
            &first_url,
            &runtime,
        )?;
        let rotated_capture = capture_snapshot(
            &view,
            rotated,
            &first_url,
            SemanticSnapshotGeneration::INITIAL,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        verify_first_snapshot(&rotated_capture.snapshot)?;
        let rotated_generation = rotated_capture.snapshot.generation();
        let rotated_observation = assemble_observation(rotated_capture)?;
        let rotated_fill = execute_primary_fill(
            &view,
            &rotated_observation,
            &runtime,
            "Semantic fill text",
            "Quasar fixed text",
            5,
            5,
        )?;
        wait_for_action_security_settle(&runtime, ACTION_SECURITY_SETTLE)?;
        let rotated_after = capture_snapshot(
            &view,
            rotated,
            &first_url,
            rotated_generation.next().ok_or("action_identity")?,
            &mut next_invocation,
            &mut successful_snapshots,
            &runtime,
        )?;
        verify_primary_fill_execution(rotated_fill, &rotated_after.snapshot)?;
        verify_primary_fill(
            &rotated_after.snapshot,
            "Semantic fill text",
            "Quasar fixed text",
            SemanticRole::Textbox,
            1,
        )?;
        registry
            .acknowledge_observation(identity.id(), rotated)
            .map_err(|_| "rotated_observation")?;

        if callbacks.failed() {
            return Err("callback_verification");
        }
        if native_guard.failed() {
            return Err("native_state_verification");
        }
        if view.semantic_pending_for_audit() != Some(false) {
            return Err("semantic_pending_verification");
        }
        if view
            .attest(profile, ContextProfileStorageClass::Ephemeral, Some(&store))
            .is_err()
        {
            return Err("view_attestation_verification");
        }
        if !server.semantic_mutation_completed() {
            return Err("mutation_verification");
        }
        if !server.is_healthy() {
            return Err("fixture_verification");
        }
        let expected_snapshots = if hostile_relay_probe { 22 } else { 18 };
        if successful_snapshots != expected_snapshots {
            return Err("snapshot_count_verification");
        }
        Ok(None)
    })();

    // Every successful mode, including early model-workflow completion, must
    // prove the owned view is still isolated, CPU-throttled and callback-drained
    // before retirement. The teardown below also runs after every refusal.
    let execution = execution.and_then(|terminal| {
        view.attest(profile, ContextProfileStorageClass::Ephemeral, Some(&store))
            .map_err(|_| "view_attestation_verification")?;
        if view.semantic_pending_for_audit() != Some(false) {
            return Err("semantic_pending_verification");
        }
        Ok(terminal)
    });

    // Stop the fixed listener before cancelling the native page so an
    // in-flight favicon or subresource socket is cancelled under the fixture's
    // explicit stop state rather than misclassified as a worker fault.
    let fixture_failure = server.shutdown().err().map(fixture_failure_stage);
    super::stop_loading(view.view());
    let policy_clean = policy.retire().is_ok();
    let semantic_clean = view.retire_semantic_runtime();
    let page_weak = Weak::from_retained(&page);
    let window_weak = Weak::from_retained(&window);
    let store_weak = Weak::from_retained(&store);
    drop(page);
    drop(view);
    window.close();
    drop(host);
    drop(content);
    drop(window);
    drop(store);
    drop(run_loop);
    drop(registry);

    let execution = if policy_clean && semantic_clean {
        execution
    } else {
        Err("runtime_retire")
    };
    Ok(PendingTeardown {
        execution,
        page: page_weak,
        window: window_weak,
        store: store_weak,
        fixture_failure,
    })
}

fn finish(
    pending: PendingTeardown,
) -> Result<Option<MacosAgenticSemanticModelClickTerminal>, &'static str> {
    let run_loop = NSRunLoop::mainRunLoop();
    let deadline = Instant::now()
        .checked_add(TEARDOWN_TIMEOUT)
        .ok_or("native_teardown")?;
    while !native_owners_drained(&pending) && Instant::now() < deadline {
        pump_once(&run_loop, None);
    }
    let owners_drained = native_owners_drained(&pending);
    if let Some(failure) = pending.fixture_failure {
        return Err(failure);
    }
    if !owners_drained {
        return Err("native_teardown");
    }
    pending.execution
}

fn fixture_failure_stage(failure: FixtureServerError) -> &'static str {
    match failure {
        FixtureServerError::Io(_) => "fixture_start_io",
        FixtureServerError::NonLoopbackBind => "fixture_bind",
        FixtureServerError::WorkerFailed => "fixture_worker",
        FixtureServerError::NonLoopbackPeer => "fixture_peer",
        FixtureServerError::RequestBudgetExhausted => "fixture_budget",
        FixtureServerError::SemanticMutationInvariant => "fixture_mutation_gate",
        FixtureServerError::SemanticLocationInvariant => "fixture_location_gate",
        FixtureServerError::AcceptFailed => "fixture_accept",
        FixtureServerError::WorkerPanicked => "fixture_panic",
    }
}

fn native_owners_drained(pending: &PendingTeardown) -> bool {
    pending.page.load().is_none()
        && pending.window.load().is_none()
        && pending.store.load().is_none()
}

fn navigate(
    view: &mut AgentOwnedView,
    registry: &mut ContextRegistry,
    id: ContextId,
    operation_id: u64,
    url: &str,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<zephium_agentic::ContextJoin, &'static str> {
    navigate_with_receipt(view, registry, id, operation_id, url, runtime)
        .map(|(context, _)| context)
}

fn navigate_with_receipt(
    view: &mut AgentOwnedView,
    registry: &mut ContextRegistry,
    id: ContextId,
    operation_id: u64,
    url: &str,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<
    (
        zephium_agentic::ContextJoin,
        zephium_agentic::ContextOperationJoin,
    ),
    &'static str,
> {
    if runtime.failed() || runtime.callbacks.navigation.borrow().is_some() {
        return Err("navigation_state");
    }
    let operation = registry
        .begin_navigation(
            id,
            ContextOperationId::new(operation_id).ok_or("navigation_state")?,
        )
        .map_err(|_| "navigation_state")?;
    let target = ContextNavigationTarget::parse(url).map_err(|_| "navigation_target")?;
    if view.prepare_semantic_document_load().is_err() {
        let _ = registry.settle_navigation(id, operation, ContextSettlement::Refused);
        return Err("semantic_epoch");
    }
    let terminal_claimed = Arc::new(AtomicBool::new(false));
    if view
        .navigation()
        .arm(operation, target.clone(), Arc::clone(&terminal_claimed))
        .is_err()
    {
        let _ = registry.settle_navigation(id, operation, ContextSettlement::Refused);
        return Err("navigation_arm");
    }
    if view.view().load_url(url).is_err() {
        let _ = view.navigation().disarm(operation);
        let _ = registry.settle_navigation(id, operation, ContextSettlement::Refused);
        return Err("navigation_load");
    }

    let timeout = if url == PUBLIC_DISCOVERY_PROBE_URL {
        PUBLIC_NAVIGATION_TIMEOUT
    } else {
        NAVIGATION_TIMEOUT
    };
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("navigation_timeout")?;
    while !runtime.failed()
        && runtime.callbacks.navigation.borrow().is_none()
        && Instant::now() < deadline
    {
        runtime.pump();
    }
    let terminal = runtime
        .callbacks
        .navigation
        .try_borrow_mut()
        .map_err(|_| "navigation_state")?
        .take();
    let disarmed = view.navigation().disarm(operation);
    let applied = terminal.is_some_and(|terminal| {
        terminal.operation() == operation
            && matches!(
                terminal.into_outcome(),
                Ok(AgentNavigationCommit::Web(committed)) if committed == target
            )
    });
    registry
        .settle_navigation(
            id,
            operation,
            if applied {
                ContextSettlement::Applied
            } else {
                ContextSettlement::Refused
            },
        )
        .map_err(|_| "navigation_settle")?;
    if !applied || !disarmed || runtime.failed() || !terminal_claimed.load(Ordering::Acquire) {
        return Err("navigation_terminal");
    }
    registry
        .join(id)
        .map(|context| (context, operation))
        .map_err(|_| "navigation_settle")
}

fn park_semantic_runtime(
    view: &mut AgentOwnedView,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<(), &'static str> {
    let result = Rc::new(Cell::new(None));
    let completion = result.clone();
    view.park_semantic_runtime(move |parked| completion.set(Some(parked)))
        .map_err(|_| "history_probe_park_dispatch")?;
    let deadline = Instant::now()
        .checked_add(SNAPSHOT_TIMEOUT)
        .ok_or("history_probe_park_timeout")?;
    while result.get().is_none() && !runtime.failed() && Instant::now() < deadline {
        runtime.pump();
    }
    if result.get() != Some(true) || !view.semantic_runtime_parked() || runtime.failed() {
        return Err("history_probe_park_terminal");
    }
    Ok(())
}

fn history_back_with_receipt(
    view: &mut AgentOwnedView,
    registry: &mut ContextRegistry,
    id: ContextId,
    operation_id: u64,
    expected_url: &str,
    item: &WKBackForwardListItem,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<zephium_agentic::ContextJoin, &'static str> {
    if runtime.failed() || runtime.callbacks.navigation.borrow().is_some() {
        return Err("history_probe_navigation_state");
    }
    let page = super::native_webview(view.view());
    let current =
        unsafe { page.backForwardList().currentItem() }.ok_or("history_probe_current_item")?;
    let back = unsafe { page.backForwardList().backItem() }.ok_or("history_probe_back_item")?;
    if std::ptr::eq(&*current, item) || !std::ptr::eq(&*back, item) {
        return Err("history_probe_pre_dispatch_identity");
    }
    let ticket = view
        .prepare_history_back()
        .map_err(|_| "history_probe_back_authority")?;
    park_semantic_runtime(view, runtime)?;
    let reactivated_target = view
        .reactivate_history_destination(ticket)
        .map_err(|_| "history_probe_reactivate")?;
    let operation = registry
        .begin_navigation(
            id,
            ContextOperationId::new(operation_id).ok_or("history_probe_navigation_state")?,
        )
        .map_err(|_| "history_probe_navigation_state")?;
    let target = ContextNavigationTarget::parse(expected_url)
        .map_err(|_| "history_probe_navigation_target")?;
    if reactivated_target != target {
        let _ = registry.settle_navigation(id, operation, ContextSettlement::Refused);
        let _ = view.refuse_history_back(ticket);
        return Err("history_probe_back_target");
    }
    let terminal_claimed = Arc::new(AtomicBool::new(false));
    if view
        .navigation()
        .arm(operation, target.clone(), Arc::clone(&terminal_claimed))
        .is_err()
    {
        let _ = registry.settle_navigation(id, operation, ContextSettlement::Refused);
        return Err("history_probe_navigation_arm");
    }
    if !view.dispatch_history_back(ticket) {
        let _ = view.navigation().disarm(operation);
        let _ = registry.settle_navigation(id, operation, ContextSettlement::Refused);
        let _ = view.refuse_history_back(ticket);
        return Err("history_probe_native_dispatch");
    }
    let deadline = Instant::now()
        .checked_add(NAVIGATION_TIMEOUT)
        .ok_or("history_probe_navigation_timeout")?;
    while !runtime.failed()
        && runtime.callbacks.navigation.borrow().is_none()
        && Instant::now() < deadline
    {
        runtime.pump();
    }
    let terminal = runtime
        .callbacks
        .navigation
        .try_borrow_mut()
        .map_err(|_| "history_probe_navigation_state")?
        .take();
    let disarmed = view.navigation().disarm(operation);
    let applied = terminal.is_some_and(|terminal| {
        terminal.operation() == operation
            && matches!(
                terminal.into_outcome(),
                Ok(AgentNavigationCommit::Web(committed)) if committed == target
            )
    });
    registry
        .settle_navigation(
            id,
            operation,
            if applied {
                ContextSettlement::Applied
            } else {
                ContextSettlement::Refused
            },
        )
        .map_err(|_| "history_probe_navigation_settle")?;
    if !applied || !disarmed || runtime.failed() || !terminal_claimed.load(Ordering::Acquire) {
        let _ = view.refuse_history_back(ticket);
        return Err("history_probe_navigation_terminal");
    }
    view.settle_history_back(ticket)
        .map_err(|_| "history_probe_history_settle")?;
    registry
        .join(id)
        .map_err(|_| "history_probe_navigation_settle")
}

fn native_current_history_item(
    page: &WKWebView,
) -> Result<Retained<WKBackForwardListItem>, &'static str> {
    unsafe { page.backForwardList().currentItem() }.ok_or("history_probe_current_item")
}

fn native_item_url(item: &WKBackForwardListItem) -> Option<String> {
    let url = unsafe { item.URL() };
    url.absoluteString().map(|value| value.to_string())
}

fn evaluate_history_probe(
    page: &WKWebView,
    source: &str,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<String, &'static str> {
    let result = Rc::new(RefCell::new(None));
    let callback_result = Rc::clone(&result);
    let completion: block2::RcBlock<dyn Fn(*mut AnyObject, *mut NSError)> =
        block2::RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            let value = if error.is_null() {
                unsafe { value.as_ref() }
                    .and_then(AnyObject::downcast_ref::<NSString>)
                    .map(ToString::to_string)
            } else {
                None
            };
            callback_result.replace(Some(value));
        });
    let source = NSString::from_str(source);
    unsafe { page.evaluateJavaScript_completionHandler(&source, Some(&completion)) };
    let deadline = Instant::now()
        .checked_add(SNAPSHOT_TIMEOUT)
        .ok_or("history_probe_evaluation_timeout")?;
    while result.borrow().is_none() && !runtime.failed() && Instant::now() < deadline {
        runtime.pump();
    }
    if runtime.failed() || Instant::now() >= deadline {
        return Err("history_probe_evaluation_timeout");
    }
    let value = result
        .borrow_mut()
        .take()
        .flatten()
        .ok_or("history_probe_evaluation");
    value
}

fn install_history_lifecycle_witness(
    page: &WKWebView,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<(), &'static str> {
    let result = evaluate_history_probe(
        page,
        "(()=>{const s={pageshow:0,persisted:false};Object.defineProperty(globalThis,'__zephiumHistoryWitness',{value:s,configurable:false});addEventListener('pageshow',e=>{s.pageshow+=1;s.persisted=Boolean(e.persisted);},{capture:true});return 'armed';})()",
        runtime,
    )?;
    (result == "armed")
        .then_some(())
        .ok_or("history_probe_witness_install")
}

fn read_history_lifecycle_witness(
    page: &WKWebView,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<[bool; 3], &'static str> {
    let encoded = evaluate_history_probe(
        page,
        "(()=>{const s=globalThis.__zephiumHistoryWitness;return JSON.stringify({resident:Boolean(s),pageshow:s?.pageshow||0,persisted:Boolean(s?.persisted)});})()",
        runtime,
    )?;
    let value: serde_json::Value =
        serde_json::from_str(&encoded).map_err(|_| "history_probe_witness_decode")?;
    Ok([
        value.get("resident").and_then(serde_json::Value::as_bool) == Some(true),
        value
            .get("pageshow")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            > 0,
        value.get("persisted").and_then(serde_json::Value::as_bool) == Some(true),
    ])
}

fn assemble_observation(
    capture: CapturedSnapshot,
) -> Result<zephium_agentic::SemanticObservation, &'static str> {
    let boundaries = capture
        .snapshot
        .nodes()
        .iter()
        .filter(|node| node.role() == SemanticRole::FrameBoundary)
        .map(SemanticNode::reference)
        .collect::<Vec<_>>();
    let mut assembler = SemanticObservationAssembler::new(capture.request, capture.snapshot)
        .map_err(|_| "action_observation")?;
    for boundary in boundaries {
        assembler
            .mark_frame_unsupported(
                FrameId::MAIN,
                boundary,
                SemanticFrameUnsupported::PlatformIsolationUnavailable,
            )
            .map_err(|_| "action_observation")?;
    }
    assembler.finish().map_err(|_| "action_observation")
}

fn model_probe_authority(
    observation: &zephium_agentic::SemanticObservation,
) -> Result<MacosAgenticSemanticProbeAuthority, &'static str> {
    model_probe_authority_with_budget(observation, 3)
}

fn model_probe_authority_with_budget(
    observation: &zephium_agentic::SemanticObservation,
    operations: u32,
) -> Result<MacosAgenticSemanticProbeAuthority, &'static str> {
    let snapshot = observation
        .frames()
        .first()
        .ok_or("model_authority_snapshot")?;
    let frame = snapshot.frame().clone();
    if observation.frames().len() != 1 || observation.request().context() != frame.context() {
        return Err("model_authority_observation");
    }
    let identity = frame.context().identity();
    let profile = identity.profile();
    let account_scope = AgentAccountScope::Anonymous;
    let origin = frame.origin().clone();
    let effects =
        AgentEffectScope::try_new(&[SemanticEffectClass::Read, SemanticEffectClass::LocalWrite])
            .map_err(|_| "model_authority_effects")?;
    let budget = AgentRunBudget::try_new(
        operations,
        MODEL_PROBE_MODEL_TOKEN_BUDGET,
        MODEL_PROBE_COST_BUDGET_MICRO_USD,
        1,
    )
    .map_err(|_| "model_authority_budget")?;
    let scope = AgentRunScope::try_new(
        vec![profile],
        vec![account_scope],
        vec![origin.clone()],
        // The semantic policy conservatively upgrades page-controlled labels
        // and redacted secret controls to sensitive taint. This explicit live
        // probe authorizes that fixed loopback fixture only; secret values are
        // still mechanically absent from the model projection.
        SemanticSensitivity::Sensitive,
        effects,
        Vec::new(),
    )
    .map_err(|_| "model_authority_scope")?;
    let node = AgentPlanNodeId::generate();
    let authority = AgentPlanNodeAuthority::try_new(
        vec![profile],
        vec![account_scope],
        vec![origin],
        SemanticSensitivity::Sensitive,
        effects,
    )
    .map_err(|_| "model_authority_node")?;
    let expires_at = AgentPolicyInstant::from_millis(MODEL_PROBE_POLICY_EXPIRES_MILLIS);
    let manifest = AgentRunManifest::try_new(
        AgentRunManifestId::generate(),
        identity.owner(),
        scope,
        budget,
        AgentPolicyInstant::from_millis(MODEL_PROBE_POLICY_NOW_MILLIS),
        expires_at,
        vec![AgentPlanNodeScope::new(
            node,
            authority,
            budget,
            AgentPolicyInstant::from_millis(MODEL_PROBE_POLICY_EXPIRES_MILLIS - 1),
        )],
    )
    .map_err(|_| "model_authority_manifest")?;
    let lease = AgentPlanLeaseBinding::new(AgentPlanLeaseId::generate(), node);
    let account = AgentContextAccountBinding::new(
        AgentAccountAttestationId::generate(),
        frame.context(),
        account_scope,
        AgentPolicyInstant::from_millis(MODEL_PROBE_POLICY_NOW_MILLIS),
    );
    Ok(MacosAgenticSemanticProbeAuthority {
        manifest,
        lease,
        account,
        observation: observation.request().clone(),
        frame,
        invocation: snapshot.invocation(),
        snapshot_generation: snapshot.generation(),
        now: AgentPolicyInstant::from_millis(MODEL_PROBE_POLICY_NOW_MILLIS + 1),
    })
}

fn execute_primary_click(
    view: &AgentOwnedView,
    observation: &zephium_agentic::SemanticObservation,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<PendingPrimaryClick, &'static str> {
    let target = observation
        .frames()
        .iter()
        .flat_map(|snapshot| snapshot.nodes())
        .find(|node| node_name_is(node, "Primary semantic action"))
        .map(SemanticNode::reference)
        .ok_or("action_target")?;
    let requested_at = SemanticActionExecutionInstant::from_millis(10_000);
    let mut execution =
        SemanticClickQualificationExecution::prepare(observation, target, 1, 1, requested_at)
            .map_err(|_| "action_prepare")?;
    let request = execution
        .take_native_request()
        .map_err(|_| "action_prepare")?;
    let admitted_at = Instant::now();
    let result = Rc::new(RefCell::new(None));
    let completion = Rc::clone(&result);
    view.dispatch_semantic_action(request, admitted_at, move |settlement| {
        if let Ok(mut slot) = completion.try_borrow_mut() {
            if slot.is_none() {
                *slot = Some(settlement);
            }
        }
    });
    let deadline = Instant::now()
        .checked_add(SNAPSHOT_TIMEOUT)
        .ok_or("action_timeout")?;
    while result.borrow().is_none() && !runtime.failed() && Instant::now() < deadline {
        runtime.pump();
    }
    if runtime.failed() {
        return Err("action_native_state");
    }
    let settlement = result
        .try_borrow_mut()
        .map_err(|_| "action_state")?
        .take()
        .ok_or("action_timeout")?;
    runtime.native_guard.sample();
    if runtime.failed() {
        return Err("action_native_state");
    }
    Ok(PendingPrimaryClick {
        execution,
        settlement,
        admitted_at,
    })
}

fn execute_model_action(
    view: &AgentOwnedView,
    request: zephium_agentic::SemanticActionNativeRequest,
    expected_kind: Option<zephium_agentic::SemanticActionKind>,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<PendingModelAction, &'static str> {
    if expected_kind.is_some_and(|kind| request.kind() != kind) {
        return Err("model_action_kind");
    }
    let wait = request.wait();
    let settle_millis = request.settle_budget().millis();
    let admitted_at = Instant::now();
    let result = Rc::new(RefCell::new(None));
    let completion = Rc::clone(&result);
    view.dispatch_semantic_action(request, admitted_at, move |settlement| {
        if let Ok(mut slot) = completion.try_borrow_mut() {
            if slot.is_none() {
                *slot = Some(settlement);
            }
        }
    });
    let deadline = Instant::now()
        .checked_add(SNAPSHOT_TIMEOUT)
        .ok_or("model_action_timeout")?;
    while result.borrow().is_none() && !runtime.failed() && Instant::now() < deadline {
        runtime.pump();
    }
    if runtime.failed() {
        return Err("model_action_native_state");
    }
    let settlement = result
        .try_borrow_mut()
        .map_err(|_| "model_action_state")?
        .take()
        .ok_or("model_action_callback_timeout")?;
    runtime.native_guard.sample();
    if runtime.failed() {
        return Err("model_action_native_state");
    }
    Ok(PendingModelAction {
        settlement,
        admitted_at,
        wait,
        settle_millis,
    })
}

fn model_action_settle_delay(
    wait: zephium_agentic::SemanticWaitCondition,
    settle_millis: u32,
) -> Result<Duration, &'static str> {
    let millis = match wait {
        zephium_agentic::SemanticWaitCondition::MutationQuiet(quiet) => quiet.millis(),
        _ => {
            u32::try_from(ACTION_SECURITY_SETTLE.as_millis()).map_err(|_| "model_action_settle")?
        }
    };
    if millis > settle_millis {
        return Err("model_action_settle_contract");
    }
    Ok(Duration::from_millis(u64::from(millis)))
}

fn execute_primary_fill(
    view: &AgentOwnedView,
    observation: &zephium_agentic::SemanticObservation,
    runtime: &ProbeRuntime<'_, '_>,
    target_name: &str,
    value: &str,
    batch: u64,
    attempt: u64,
) -> Result<PendingPrimaryFill, &'static str> {
    let target = observation
        .frames()
        .iter()
        .flat_map(|snapshot| snapshot.nodes())
        .find(|node| node_name_is(node, target_name))
        .map(SemanticNode::reference)
        .ok_or("fill_target")?;
    let value = SemanticActionText::try_new(value.to_owned()).map_err(|_| "fill_value")?;
    let requested_at = SemanticActionExecutionInstant::from_millis(10_000);
    let mut execution = SemanticFillQualificationExecution::prepare(
        observation,
        target,
        value,
        batch,
        attempt,
        requested_at,
    )
    .map_err(|_| "fill_prepare")?;
    let request = execution
        .take_native_request()
        .map_err(|_| "fill_prepare")?;
    let admitted_at = Instant::now();
    let result = Rc::new(RefCell::new(None));
    let completion = Rc::clone(&result);
    let deadline = Instant::now()
        .checked_add(SNAPSHOT_TIMEOUT)
        .ok_or("fill_timeout")?;
    let preparation_authority = Rc::new(std::cell::Cell::new(true));
    let checked_preparation = preparation_authority.clone();
    view.dispatch_retained_semantic_action(
        request,
        admitted_at,
        Box::new(move || checked_preparation.get() && Instant::now() < deadline),
        move |settlement| {
            if let Ok(mut slot) = completion.try_borrow_mut() {
                if slot.is_none() {
                    *slot = Some(settlement);
                }
            }
        },
    );
    while result.borrow().is_none() && !runtime.failed() && Instant::now() < deadline {
        runtime.pump();
        preparation_authority.set(!runtime.failed() && Instant::now() < deadline);
        if let Some(semantic) = view.semantic() {
            semantic.poll_prepared_fill();
        }
    }
    preparation_authority.set(false);
    if runtime.failed() {
        return Err("fill_native_state");
    }
    let settlement = result
        .try_borrow_mut()
        .map_err(|_| "fill_state")?
        .take()
        .ok_or("fill_timeout")?;
    runtime.native_guard.sample();
    if runtime.failed() {
        return Err("fill_native_state");
    }
    Ok(PendingPrimaryFill {
        execution,
        settlement,
        admitted_at,
    })
}

fn verify_primary_click_execution(
    pending: PendingPrimaryClick,
    snapshot: &SemanticSnapshot,
) -> Result<(), &'static str> {
    let observed_at = action_observed_at(pending.admitted_at)?;
    let applied = pending
        .execution
        .settle_and_verify(pending.settlement, snapshot, observed_at)
        .map_err(|_| "action_verification")?;
    if applied.backend() != SemanticActionExecutionBackend::FixedSemanticRecipe
        || applied.readiness() != SemanticActionNativeReadiness::ExactVisibleUnoccludedTarget
        || applied.completed_at() > SemanticActionExecutionInstant::from_millis(11_000)
    {
        return Err("action_evidence");
    }
    Ok(())
}

fn action_observed_at(admitted_at: Instant) -> Result<SemanticSettleInstant, &'static str> {
    let elapsed = u64::try_from(admitted_at.elapsed().as_millis())
        .map_err(|_| "action_verification_clock")?;
    10_000_u64
        .checked_add(elapsed)
        .map(SemanticSettleInstant::from_millis)
        .ok_or("action_verification_clock")
}

fn verify_primary_fill_execution(
    pending: PendingPrimaryFill,
    snapshot: &SemanticSnapshot,
) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("fill_verification_incomplete_snapshot");
    }
    if let Some(failure) = pending.settlement.qualification_failure() {
        return Err(match failure {
            zephium_agentic::SemanticActionNativeFailure::StaleReference => {
                "fill_native_stale_reference"
            }
            zephium_agentic::SemanticActionNativeFailure::TargetChanged => {
                "fill_native_target_changed"
            }
            zephium_agentic::SemanticActionNativeFailure::TargetDisabled => {
                "fill_native_target_disabled"
            }
            zephium_agentic::SemanticActionNativeFailure::CredentialBoundary => {
                "fill_native_credential_boundary"
            }
            zephium_agentic::SemanticActionNativeFailure::TargetOccluded => {
                "fill_native_target_occluded"
            }
            zephium_agentic::SemanticActionNativeFailure::UnsupportedInteraction => {
                "fill_native_unsupported"
            }
            zephium_agentic::SemanticActionNativeFailure::NeedsHuman => "fill_native_needs_human",
            zephium_agentic::SemanticActionNativeFailure::AppliedUnverified => {
                "fill_native_applied_unverified"
            }
            zephium_agentic::SemanticActionNativeFailure::RendererLost => {
                "fill_native_renderer_lost"
            }
            zephium_agentic::SemanticActionNativeFailure::TimedOut => "fill_native_timed_out",
            zephium_agentic::SemanticActionNativeFailure::Cancelled => "fill_native_cancelled",
            zephium_agentic::SemanticActionNativeFailure::ResourceExhausted => {
                "fill_native_resource_exhausted"
            }
            zephium_agentic::SemanticActionNativeFailure::Transport => "fill_native_transport",
            zephium_agentic::SemanticActionNativeFailure::Shutdown => "fill_native_shutdown",
        });
    }
    let elapsed = u64::try_from(pending.admitted_at.elapsed().as_millis())
        .map_err(|_| "fill_verification_clock")?;
    let observed_at = 10_000_u64
        .checked_add(elapsed)
        .map(SemanticSettleInstant::from_millis)
        .ok_or("fill_verification_clock")?;
    let applied = pending
        .execution
        .settle_and_verify(pending.settlement, snapshot, observed_at)
        .map_err(|failure| match failure {
            SemanticActionQualificationError::Identity => "fill_verification_identity",
            SemanticActionQualificationError::Contract => "fill_verification_contract",
            SemanticActionQualificationError::Binding(_) => "fill_verification_binding",
            SemanticActionQualificationError::Checkpoint(_) => "fill_verification_checkpoint",
            SemanticActionQualificationError::NativeAdmission(_) => {
                "fill_verification_native_admission"
            }
            SemanticActionQualificationError::RequestAlreadyTaken => "fill_verification_request",
            SemanticActionQualificationError::Terminal => "fill_verification_terminal",
            SemanticActionQualificationError::Settlement => "fill_verification_settlement",
            SemanticActionQualificationError::Verification => "fill_verification_effect",
        })?;
    if applied.backend() != SemanticActionExecutionBackend::FixedSemanticRecipe
        || applied.readiness() != SemanticActionNativeReadiness::ExactConnectedWritableFormTarget
        || applied.completed_at() > SemanticActionExecutionInstant::from_millis(11_000)
    {
        return Err("fill_evidence");
    }
    Ok(())
}

fn verify_hostile_fill_refusal(
    pending: PendingPrimaryFill,
    snapshot: &SemanticSnapshot,
) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("hostile_fill_incomplete_snapshot");
    }
    if pending.settlement.qualification_failure()
        != Some(zephium_agentic::SemanticActionNativeFailure::AppliedUnverified)
    {
        return Err("hostile_fill_not_indeterminate");
    }
    let elapsed = u64::try_from(pending.admitted_at.elapsed().as_millis())
        .map_err(|_| "hostile_fill_clock")?;
    let observed_at = 10_000_u64
        .checked_add(elapsed)
        .map(SemanticSettleInstant::from_millis)
        .ok_or("hostile_fill_clock")?;
    if !matches!(
        pending
            .execution
            .settle_and_verify(pending.settlement, snapshot, observed_at),
        Err(SemanticActionQualificationError::Settlement)
    ) {
        return Err("hostile_fill_retryable_terminal");
    }
    let expected = "Semantic hostile relay refused untrusted target unchanged recovery unchanged type restored target-marker clear recovery-marker missing popup denied activation during inactive sticky inactive settle inactive sticky inactive";
    if !snapshot_contains(snapshot, expected) {
        return Err("hostile_fill_security_evidence");
    }
    if !snapshot_value_is(snapshot, "Semantic hostile fill", "hostile-before") {
        return Err("hostile_fill_target_mutated");
    }
    if !snapshot_value_is(snapshot, "Semantic hostile recovery", "recovery-before") {
        return Err("hostile_fill_cross_node_mutated");
    }
    Ok(())
}

fn verify_hostile_fill_recovery(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if !snapshot_value_is(snapshot, "Semantic hostile fill", "hostile-before")
        || !snapshot_contains(
            snapshot,
            "Semantic hostile relay recovered target-marker clear recovery-marker clear",
        )
    {
        return Err("hostile_fill_recovery_evidence");
    }
    Ok(())
}

fn verify_hostile_credential_refusal(
    pending: PendingPrimaryFill,
    snapshot: &SemanticSnapshot,
) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("hostile_credential_incomplete_snapshot");
    }
    if pending.settlement.qualification_failure()
        != Some(zephium_agentic::SemanticActionNativeFailure::AppliedUnverified)
    {
        return Err("hostile_credential_not_indeterminate");
    }
    let elapsed = u64::try_from(pending.admitted_at.elapsed().as_millis())
        .map_err(|_| "hostile_credential_clock")?;
    let observed_at = 10_000_u64
        .checked_add(elapsed)
        .map(SemanticSettleInstant::from_millis)
        .ok_or("hostile_credential_clock")?;
    if !matches!(
        pending
            .execution
            .settle_and_verify(pending.settlement, snapshot, observed_at),
        Err(SemanticActionQualificationError::Settlement)
    ) {
        return Err("hostile_credential_retryable_terminal");
    }
    if !snapshot_value_is(
        snapshot,
        "Semantic hostile credential fill",
        "credential-before",
    ) {
        return Err("hostile_credential_value_mutated");
    }
    if !snapshot_contains(snapshot, "Semantic hostile credential recovery pending") {
        return Err("hostile_credential_input_observed");
    }
    let expected = "Semantic hostile credential refused untrusted target unchanged credential-marker clear target-marker clear popup denied activation during inactive sticky inactive settle inactive sticky inactive";
    if !snapshot_contains(snapshot, expected) {
        return Err("hostile_credential_security_evidence");
    }
    Ok(())
}

fn verify_hostile_credential_recovery(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if !snapshot_value_is(
        snapshot,
        "Semantic hostile credential fill",
        "Quasar credential recovery",
    ) || !snapshot_contains(
        snapshot,
        "Semantic hostile credential recovered input untrusted value exact credential-marker clear target-marker clear",
    ) {
        return Err("hostile_credential_recovery_evidence");
    }
    Ok(())
}

fn snapshot_value_is(snapshot: &SemanticSnapshot, name: &str, expected: &str) -> bool {
    snapshot
        .nodes()
        .iter()
        .find(|node| node_name_is(node, name))
        .and_then(SemanticNode::value)
        .is_some_and(|value| match value {
            SemanticValueSummary::Text(value) => {
                let preview = value.preview();
                !preview.truncated()
                    && preview.source_bytes() == expected.len()
                    && preview.text() == expected
            }
            _ => false,
        })
}

fn verify_public_discovery_snapshot(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("public_discovery_snapshot");
    }
    if !snapshot
        .nodes()
        .iter()
        .any(|node| matches!(node.role(), SemanticRole::Textbox | SemanticRole::Searchbox))
    {
        return Err("public_discovery_textbox_missing");
    }
    if !snapshot
        .nodes()
        .iter()
        .any(|node| node.operations().contains(SemanticOperationClass::Select))
    {
        return Err("public_discovery_select_missing");
    }
    if !snapshot
        .nodes()
        .iter()
        .any(|node| node.role() == SemanticRole::Option && node_name_is(node, "Deutsch"))
    {
        return Err("public_discovery_option_missing");
    }
    Ok(())
}

fn wait_for_action_security_settle(
    runtime: &ProbeRuntime<'_, '_>,
    duration: Duration,
) -> Result<(), &'static str> {
    let deadline = Instant::now()
        .checked_add(duration)
        .ok_or("action_settle")?;
    while !runtime.failed() && Instant::now() < deadline {
        runtime.pump();
    }
    if runtime.failed() {
        Err("action_native_state")
    } else {
        Ok(())
    }
}

fn capture_snapshot(
    view: &AgentOwnedView,
    context: zephium_agentic::ContextJoin,
    url: &str,
    first_generation: SemanticSnapshotGeneration,
    next_invocation: &mut u64,
    successful_snapshots: &mut u8,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<CapturedSnapshot, &'static str> {
    let origin = SemanticOrigin::parse(url).map_err(|_| "snapshot_origin")?;
    let frame = SemanticFrameJoin::try_new(
        context,
        FrameId::MAIN,
        context.frame_generation(),
        origin,
        SemanticFrameTrust::SameOrigin,
    )
    .map_err(|_| "snapshot_frame")?;
    let deadline = Instant::now()
        .checked_add(SNAPSHOT_TIMEOUT)
        .ok_or("snapshot_timeout")?;

    for retry in 0..MAX_DOCUMENT_LOADING_RETRIES {
        if runtime.failed() || Instant::now() >= deadline {
            return Err("snapshot_loading_timeout");
        }
        let invocation_value = take_semantic_identity(next_invocation)?;
        let generation = first_generation
            .get()
            .checked_add(u64::from(retry))
            .and_then(SemanticSnapshotGeneration::new)
            .ok_or("snapshot_identity")?;
        let request = SemanticObservationRequest::initial(
            SemanticObservationId::new(invocation_value).ok_or("snapshot_identity")?,
            context,
            SemanticObservationBudget::INITIAL_FILTERED,
        );
        let invocation = encode_semantic_runtime_invocation(
            &request,
            frame.clone(),
            SemanticInvocationId::new(invocation_value).ok_or("snapshot_identity")?,
            generation,
            SemanticRuntimeBudget::INITIAL_FILTERED,
        )
        .map_err(|_| "snapshot_encode")?;
        let outcome = dispatch_invocation(view, invocation, runtime, deadline)?;
        match outcome {
            Ok(snapshot) => {
                *successful_snapshots = successful_snapshots
                    .checked_add(1)
                    .ok_or("snapshot_count")?;
                return Ok(CapturedSnapshot { request, snapshot });
            }
            Err(SemanticRuntimePortFailure::Result(SemanticRuntimeResultError::Runtime(
                SemanticRuntimeFault::DocumentLoading,
            ))) => runtime.pump(),
            Err(_) => return Err("snapshot_result"),
        }
    }
    Err("snapshot_loading_timeout")
}

fn dispatch_invocation(
    view: &AgentOwnedView,
    invocation: SemanticRuntimeInvocation,
    runtime: &ProbeRuntime<'_, '_>,
    deadline: Instant,
) -> Result<Result<SemanticSnapshot, SemanticRuntimePortFailure>, &'static str> {
    if runtime.failed() || Instant::now() >= deadline {
        return Err("snapshot_timeout");
    }
    let result = Rc::new(RefCell::new(None));
    let completion_failed = Rc::new(Cell::new(false));
    let completion = Rc::clone(&result);
    let completion_failure = Rc::clone(&completion_failed);
    view.dispatch_semantic(invocation, move |outcome| {
        let Ok(mut slot) = completion.try_borrow_mut() else {
            completion_failure.set(true);
            return;
        };
        if slot.is_some() {
            completion_failure.set(true);
        } else {
            *slot = Some(outcome);
        }
    })
    .map_err(|_| "snapshot_dispatch")?;
    while result.borrow().is_none()
        && !completion_failed.get()
        && !runtime.failed()
        && Instant::now() < deadline
    {
        runtime.pump();
    }
    if completion_failed.get() {
        return Err("snapshot_completion_state");
    }
    if let Some(stage) = runtime.failure_stage() {
        if let Some(Err(failure)) = result.borrow().as_ref() {
            eprintln!("semantic-fixture-closed-failure: {failure:?}");
        }
        return Err(stage);
    }
    let outcome = result
        .try_borrow_mut()
        .map_err(|_| "snapshot_state")?
        .take()
        .ok_or("snapshot_callback_timeout")?;
    Ok(outcome)
}

fn take_semantic_identity(next: &mut u64) -> Result<u64, &'static str> {
    let identity = *next;
    if identity == 0 {
        return Err("snapshot_identity");
    }
    *next = identity.checked_add(1).ok_or("snapshot_identity")?;
    Ok(identity)
}

fn wait_for_mutation_gate(
    server: &FixtureServer,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<(), &'static str> {
    let deadline = Instant::now()
        .checked_add(MUTATION_GATE_TIMEOUT)
        .ok_or("mutation_timeout")?;
    while !server.semantic_mutation_waiting()
        && server.is_healthy()
        && !runtime.failed()
        && Instant::now() < deadline
    {
        runtime.pump();
    }
    if !server.is_healthy() {
        return Err("mutation_fixture");
    }
    if runtime.failed() {
        return Err("mutation_native_state");
    }
    if !server.semantic_mutation_waiting() {
        return Err("mutation_gate_timeout");
    }
    Ok(())
}

fn wait_for_mutation_application(
    server: &FixtureServer,
    runtime: &ProbeRuntime<'_, '_>,
) -> Result<(), &'static str> {
    let deadline = Instant::now()
        .checked_add(MUTATION_GATE_TIMEOUT)
        .ok_or("mutation_timeout")?;
    while !server.semantic_mutation_completed()
        && server.is_healthy()
        && !runtime.failed()
        && Instant::now() < deadline
    {
        runtime.pump();
    }
    if !server.semantic_mutation_completed() || !server.is_healthy() || runtime.failed() {
        return Err("mutation_apply");
    }

    let settle_deadline = Instant::now()
        .checked_add(MUTATION_APPLY_SETTLE)
        .ok_or("mutation_timeout")?;
    while Instant::now() < settle_deadline {
        runtime.pump();
        if runtime.failed() {
            return Err("mutation_apply");
        }
    }
    Ok(())
}

fn verify_first_snapshot(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("first_incomplete");
    }
    use zephium_agentic::SemanticFillSupport;
    for (label, support) in [
        ("Semantic fill editable", SemanticFillSupport::Supported),
        (
            "Fill support missing attribute",
            SemanticFillSupport::MissingExplicitEditable,
        ),
        (
            "Fill support unsupported tag",
            SemanticFillSupport::UnsupportedTag,
        ),
        (
            "Fill support editable ancestor",
            SemanticFillSupport::Supported,
        ),
        (
            "Fill support rich editable ancestor",
            SemanticFillSupport::EditableAncestor,
        ),
        (
            "Fill support element child",
            SemanticFillSupport::ElementChild,
        ),
        ("Fill support other child", SemanticFillSupport::OtherChild),
        ("Fill support readonly", SemanticFillSupport::ReadOnly),
        ("Fill support disabled", SemanticFillSupport::Disabled),
        (
            "Fill support unsupported control",
            SemanticFillSupport::UnsupportedControl,
        ),
        ("Fill support child limit", SemanticFillSupport::ChildLimit),
    ] {
        let node = snapshot
            .nodes()
            .iter()
            .find(|node| node.name().is_some_and(|name| name.as_str() == label))
            .ok_or("fill_support_fixture_missing")?;
        if node.fill_support() != Some(support)
            || node.operations().contains(SemanticOperationClass::Fill)
                != (support == SemanticFillSupport::Supported)
        {
            use std::io::Write as _;
            writeln!(
                std::io::stdout().lock(),
                "fill-support-fixture: expected={support:?} actual={:?} fill={} content=redacted",
                node.fill_support(),
                node.operations().contains(SemanticOperationClass::Fill)
            )
            .map_err(|_| "fill_support_diagnostic")?;
            return Err("fill_support_fixture_mismatch");
        }
        let expected_shape = match label {
            "Semantic fill editable" | "Fill support readonly" | "Fill support disabled" => {
                Some((1, 1, false))
            }
            "Fill support editable ancestor" => Some((1, 1, true)),
            "Fill support rich editable ancestor" => Some((1, 2, true)),
            "Fill support element child" => Some((1, 2, false)),
            "Fill support other child" => Some((2, 5, false)),
            "Fill support child limit" => Some((129, 1, false)),
            _ => None,
        };
        let actual_shape = node.editable_structure().map(|shape| {
            (
                shape.child_count(),
                u8::from(shape.has_text())
                    | (u8::from(shape.has_elements()) << 1)
                    | (u8::from(shape.has_other()) << 2),
                shape.editable_parent(),
            )
        });
        if actual_shape != expected_shape {
            return Err("editable_structure_fixture_mismatch");
        }
    }
    for (needle, stage) in [
        ("First semantic epoch", "first_epoch_missing"),
        ("Page bridge absent", "first_bridge_absence_missing"),
        ("Primary semantic action", "first_primary_action_missing"),
    ] {
        if !snapshot_contains(snapshot, needle) {
            return Err(stage);
        }
    }
    for (needle, stage) in [
        (
            "Closed internal must remain absent",
            "first_closed_shadow_exposed",
        ),
        ("Page bridge present", "first_page_bridge_exposed"),
        ("page-world-forgery", "first_page_world_forgery_exposed"),
        ("fixture-password-value", "first_password_exposed"),
        ("Bearer abcdefghijklmnop", "first_token_exposed"),
    ] {
        if snapshot_contains(snapshot, needle) {
            return Err(stage);
        }
    }
    let password = snapshot
        .nodes()
        .iter()
        .find(|node| node.role() == SemanticRole::Password)
        .ok_or("first_password_missing")?;
    let mut token_candidates = snapshot.nodes().iter().filter(|node| {
        node.role() == SemanticRole::Textbox
            && node.value() == Some(&SemanticValueSummary::Redacted)
            && node.sensitivity() == SemanticSensitivity::Secret
    });
    let token = token_candidates.next().ok_or("first_token_missing")?;
    if token_candidates.next().is_some() {
        return Err("first_token_ambiguous");
    }
    if password.value() != Some(&SemanticValueSummary::Redacted) {
        return Err("first_password_not_redacted");
    }
    if password.sensitivity() != SemanticSensitivity::Secret {
        return Err("first_password_not_secret");
    }
    if token.value() != Some(&SemanticValueSummary::Redacted) {
        return Err("first_token_not_redacted");
    }
    if token.sensitivity() != SemanticSensitivity::Secret {
        return Err("first_token_not_secret");
    }
    if !snapshot_contains(snapshot, "Open shadow semantic action") {
        return Err("first_open_shadow_missing");
    }
    if !snapshot.nodes().iter().any(|node| {
        node.role() == SemanticRole::Button
            && node.operations().contains(SemanticOperationClass::Click)
    }) {
        return Err("first_click_operation_missing");
    }
    for (name, role) in [
        ("Semantic fill text", SemanticRole::Textbox),
        ("Semantic fill search", SemanticRole::Searchbox),
        ("Semantic fill textarea", SemanticRole::Textbox),
        ("Semantic fill editable", SemanticRole::Textbox),
        ("Semantic hostile fill", SemanticRole::Textbox),
        ("Semantic hostile recovery", SemanticRole::Textbox),
        ("Semantic hostile credential fill", SemanticRole::Textbox),
    ] {
        let target = snapshot
            .nodes()
            .iter()
            .find(|node| node_name_is(node, name))
            .ok_or("first_fill_target_missing")?;
        if target.role() != role
            || !target.operations().contains(SemanticOperationClass::Fill)
            || target.sensitivity() == SemanticSensitivity::Secret
        {
            return Err("first_fill_operation_missing");
        }
    }
    if !snapshot
        .nodes()
        .iter()
        .any(|node| node.role() == SemanticRole::FrameBoundary)
    {
        return Err("first_frame_boundary_missing");
    }
    Ok(())
}

fn verify_primary_click(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if snapshot_contains(snapshot, "Primary semantic action applied trusted") {
        return Err("action_unexpected_trusted_input");
    }
    if !snapshot_contains(snapshot, "Primary semantic action applied untrusted") {
        return Err("action_effect_missing");
    }
    if snapshot_contains(snapshot, "popup admitted") {
        return Err("action_popup_admitted");
    }
    if !snapshot_contains(snapshot, "popup denied") {
        return Err("action_popup_evidence_missing");
    }
    if snapshot_contains(snapshot, "activation during active")
        || snapshot_contains(snapshot, "settle active")
        || snapshot_contains(snapshot, "sticky active")
    {
        return Err("action_user_activation");
    }
    if !snapshot_contains(
        snapshot,
        "Primary semantic action applied untrusted activation during inactive sticky inactive popup denied settle inactive sticky inactive",
    ) {
        return Err("action_security_evidence_missing");
    }
    Ok(())
}

fn verify_primary_fill(
    snapshot: &SemanticSnapshot,
    target_name: &str,
    expected_value: &str,
    expected_role: SemanticRole,
    completed: usize,
) -> Result<(), &'static str> {
    let target = snapshot
        .nodes()
        .iter()
        .find(|node| node_name_is(node, target_name))
        .ok_or("fill_target_missing")?;
    if target.role() != expected_role
        || !target.operations().contains(SemanticOperationClass::Fill)
        || target.states().contains(SemanticState::Focused)
        || target.sensitivity() == SemanticSensitivity::Secret
    {
        return Err("fill_target_authority");
    }
    let observed = match target.value() {
        Some(SemanticValueSummary::Text(value))
            if !value.preview().truncated()
                && value.preview().source_bytes() == value.preview().len() =>
        {
            value.preview().text()
        }
        None if expected_value.is_empty() => "",
        _ => return Err("fill_value_evidence"),
    };
    if observed != expected_value {
        return Err("fill_value_evidence");
    }
    let expected_status = "Semantic fill observed input untrusted replacement yes data exact projection exact before 1 input 1 change 0 popup denied activation during inactive sticky inactive settle inactive sticky inactive";
    if snapshot
        .nodes()
        .iter()
        .filter(|node| node_name_is(node, expected_status))
        .count()
        != completed
    {
        for (needle, stage) in [
            ("Semantic fill observed input trusted", "fill_event_trusted"),
            ("replacement no", "fill_event_input_type"),
            ("data mismatch", "fill_event_data"),
            ("projection mismatch", "fill_event_projection"),
            ("before 0", "fill_event_beforeinput"),
            ("change 1", "fill_event_change"),
            ("popup admitted", "fill_event_popup"),
            ("activation during active", "fill_event_activation"),
            ("settle active", "fill_event_settle_activation"),
        ] {
            if snapshot_contains(snapshot, needle) {
                return Err(stage);
            }
        }
        if !snapshot_contains(snapshot, "Semantic fill observed input") {
            return Err("fill_event_missing");
        }
        return Err(match completed {
            1 => "fill_text_event_evidence",
            2 => "fill_search_event_evidence",
            3 => "fill_textarea_event_evidence",
            _ => "fill_event_evidence",
        });
    }
    for forbidden in [
        "Semantic fill observed input trusted",
        "data mismatch",
        "before 0",
        "change 1",
        "popup admitted",
        "activation during active",
        "settle active",
        "sticky active",
    ] {
        if snapshot_contains(snapshot, forbidden) {
            return Err("fill_security_evidence");
        }
    }
    Ok(())
}

fn page_world_fill_relay_probe_enabled() -> bool {
    std::env::var_os(PAGE_WORLD_FILL_RELAY_PROBE_ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
}

fn page_world_fill_relay_hostile_probe_enabled() -> bool {
    std::env::var_os(PAGE_WORLD_FILL_RELAY_HOSTILE_PROBE_ENV).as_deref()
        == Some(std::ffi::OsStr::new("1"))
}

fn verify_mutation_before(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("mutation_before_incomplete");
    }
    for (needle, stage) in [
        ("Mutation semantic epoch", "mutation_epoch_missing"),
        ("Mutation stable before", "mutation_stable_before_missing"),
        ("Transient mutation anchor", "mutation_anchor_missing"),
    ] {
        if !snapshot_contains(snapshot, needle) {
            return Err(stage);
        }
    }
    let transient = snapshot
        .nodes()
        .iter()
        .find(|node| node_name_is(node, "Transient mutation anchor"))
        .ok_or("mutation_anchor_missing")?;
    if transient.role() != SemanticRole::Button
        || !transient
            .operations()
            .contains(SemanticOperationClass::Click)
    {
        return Err("mutation_anchor_authority");
    }
    if snapshot_contains(snapshot, "Mutation stable after") {
        return Err("mutation_applied_early");
    }
    Ok(())
}

fn verify_mutation_after(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("mutation_after_incomplete");
    }
    for (needle, stage) in [
        ("Mutation semantic epoch", "mutation_epoch_missing"),
        ("Mutation stable after", "mutation_stable_after_missing"),
    ] {
        if !snapshot_contains(snapshot, needle) {
            return Err(stage);
        }
    }
    for (needle, stage) in [
        ("Mutation stable before", "mutation_stale_label_exposed"),
        ("Transient mutation anchor", "mutation_stale_anchor_exposed"),
    ] {
        if snapshot_contains(snapshot, needle) {
            return Err(stage);
        }
    }
    Ok(())
}

fn verify_replacement_snapshot(snapshot: &SemanticSnapshot) -> Result<(), &'static str> {
    if snapshot.completeness() != SemanticCompleteness::Complete {
        return Err("replacement_incomplete");
    }
    for (needle, stage) in [
        ("Replacement semantic epoch", "replacement_epoch_missing"),
        ("Replacement semantic action", "replacement_action_missing"),
        ("Page bridge absent", "replacement_bridge_absence_missing"),
    ] {
        if !snapshot_contains(snapshot, needle) {
            return Err(stage);
        }
    }
    for (needle, stage) in [
        ("First semantic epoch", "replacement_stale_epoch_exposed"),
        (
            "Mutation semantic epoch",
            "replacement_mutation_epoch_exposed",
        ),
        (
            "Mutation stable before",
            "replacement_mutation_stale_exposed",
        ),
        (
            "Mutation stable after",
            "replacement_mutation_fresh_exposed",
        ),
        (
            "Transient mutation anchor",
            "replacement_mutation_anchor_exposed",
        ),
        ("Page bridge present", "replacement_page_bridge_exposed"),
        (
            "replacement-page-world-forgery",
            "replacement_page_world_forgery_exposed",
        ),
    ] {
        if snapshot_contains(snapshot, needle) {
            return Err(stage);
        }
    }
    Ok(())
}

fn node_name_is(node: &SemanticNode, expected: &str) -> bool {
    node.name().is_some_and(|name| name.as_str() == expected)
}

fn snapshot_contains(snapshot: &SemanticSnapshot, needle: &str) -> bool {
    snapshot.nodes().iter().any(|node| {
        node.name()
            .is_some_and(|value| value.as_str().contains(needle))
            || node
                .text()
                .is_some_and(|value| value.as_str().contains(needle))
            || matches!(
                node.value(),
                Some(SemanticValueSummary::Text(value)) if value.preview().text().contains(needle)
            )
    })
}

fn new_window(mtm: MainThreadMarker) -> Result<Retained<NSWindow>, &'static str> {
    // SAFETY: the marker proves AppKit main-thread affinity. The window is
    // initially ordered out and cannot release itself while the child exists.
    // Only the closed presented-rendering probe may temporarily present it.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(80.0, 80.0), NSSize::new(760.0, 640.0)),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.orderOut(None);
    window.contentView().ok_or("window_construct")?;
    Ok(window)
}

/// Qualifies an actual EngineHost port, not a directly constructed probe page.
/// The caller owns runtime/controller/audit policy; this adapter only pumps
/// the same main-thread dispatch used by the application composition root.
pub(crate) fn run_work_actor(
    profile: ProfileId,
    sink: impl Fn(zephium_agentic::ContextNativeEvent) + Send + Sync + 'static,
    start: impl FnOnce(
        Arc<dyn zephium_agentic::AgentBrowserPort>,
    ) -> Result<crate::MacosAgentWorkProbePoll, &'static str>,
) -> Result<(), &'static str> {
    run_work_host(
        profile,
        WorkProbeTeardown::Host,
        crate::MacosWorkProbeInput::LifecycleOnly,
        None,
        Duration::from_secs(180),
        move |engine| {
            let port = engine
                .take_agent_browser_port(sink)
                .ok_or("actor_port_taken")?;
            start(port)
        },
    )
}

#[cfg(feature = "agentic-browser-qa")]
pub(crate) fn run_construction_host(
    profile: ProfileId,
    start: impl FnOnce(
        Arc<dyn zephium_agentic::AgentBrowserPort>,
    ) -> Result<crate::MacosAgentWorkProbePoll, &'static str>,
) -> Result<(), &'static str> {
    run_construction_host_with_input(profile, crate::MacosWorkProbeInput::LifecycleOnly, start)
}

#[cfg(feature = "agentic-browser-qa")]
pub(crate) fn run_construction_host_with_input(
    profile: ProfileId,
    input: crate::MacosWorkProbeInput,
    start: impl FnOnce(
        Arc<dyn zephium_agentic::AgentBrowserPort>,
    ) -> Result<crate::MacosAgentWorkProbePoll, &'static str>,
) -> Result<(), &'static str> {
    run_work_host(
        profile,
        WorkProbeTeardown::ForegroundHost,
        input,
        None,
        Duration::from_secs(45),
        move |engine| {
            start(
                engine
                    .take_agent_browser_port(|_| {})
                    .ok_or("construction_port")?,
            )
        },
    )
}

/// Excluded native host for the real application composition. It hands over
/// the actual engine owner without consuming its one-shot agent port.
pub(crate) fn run_work_application(
    profile: ProfileId,
    start: impl FnOnce(
        Arc<crate::WebviewEngine>,
    ) -> Result<crate::MacosAgentWorkProbePoll, &'static str>,
) -> Result<(), &'static str> {
    run_work_host(
        profile,
        WorkProbeTeardown::Application,
        crate::MacosWorkProbeInput::LifecycleOnly,
        None,
        Duration::from_secs(180),
        start,
    )
}

pub(crate) fn run_work_application_with_events(
    profile: ProfileId,
    input: crate::MacosWorkProbeInput,
    timeout: Duration,
    events: impl Fn(crate::EngineEvent) + Send + Sync + 'static,
    start: impl FnOnce(
        Arc<crate::WebviewEngine>,
    ) -> Result<crate::MacosAgentWorkProbePoll, &'static str>,
) -> Result<(), &'static str> {
    run_work_host(
        profile,
        WorkProbeTeardown::Application,
        input,
        Some(Arc::new(events)),
        timeout,
        start,
    )
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WorkProbeTeardown {
    Host,
    Application,
    #[cfg(feature = "agentic-browser-qa")]
    ForegroundHost,
}

impl WorkProbeTeardown {
    fn host_required(self, application_succeeded: bool) -> bool {
        self != Self::Application || !application_succeeded
    }
}

#[cfg(test)]
#[test]
fn work_probe_teardown_keeps_exactly_one_success_owner_and_failure_cleanup() {
    assert!(WorkProbeTeardown::Host.host_required(true));
    assert!(WorkProbeTeardown::Host.host_required(false));
    assert!(!WorkProbeTeardown::Application.host_required(true));
    assert!(WorkProbeTeardown::Application.host_required(false));
}

/// Probe only: a hidden host's profile gets these content rules instead of
/// none, to compare a page with and without the release lists.
static PROBE_CONTENT_RULES: std::sync::OnceLock<Arc<zephium_core::blocker::ContentRules>> =
    std::sync::OnceLock::new();

pub(crate) fn use_probe_content_rules(rules: Arc<zephium_core::blocker::ContentRules>) -> bool {
    PROBE_CONTENT_RULES.set(rules).is_ok()
}

fn run_work_host(
    profile: ProfileId,
    teardown: WorkProbeTeardown,
    input: crate::MacosWorkProbeInput,
    events: Option<Arc<dyn Fn(crate::EngineEvent) + Send + Sync>>,
    timeout: Duration,
    start: impl FnOnce(
        Arc<crate::WebviewEngine>,
    ) -> Result<crate::MacosAgentWorkProbePoll, &'static str>,
) -> Result<(), &'static str> {
    use zephium_core::ports::engine::Engine as _;
    if timeout.is_zero() || timeout > Duration::from_secs(3600) {
        return Err("actor_host_timeout");
    }
    let application_policy = events.is_some();
    #[cfg(feature = "agentic-browser-qa")]
    let foreground = application_policy || teardown == WorkProbeTeardown::ForegroundHost;
    #[cfg(not(feature = "agentic-browser-qa"))]
    let foreground = application_policy;
    let mtm = MainThreadMarker::new().ok_or("actor_main_thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let activation = if foreground {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    if app.activationPolicy() != activation && !app.setActivationPolicy(activation) {
        return Err("actor_activation_policy");
    }
    app.finishLaunching();
    let mut application_events = 0_u32;
    let window = if foreground {
        // This explicit live product qualifier supplies the normal foreground
        // host required by shipping observation presentation. Legacy hidden
        // port probes keep their original focus-isolation contract.
        // SAFETY: AppKit main-thread marker owns construction and close below.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(80.0, 80.0), NSSize::new(760.0, 640.0)),
                NSWindowStyleMask::Titled,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the retained host is explicitly closed exactly once below.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("Quasar Work qualification"));
        app.activate();
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        window.makeKeyAndOrderFront(None);
        window.makeMainWindow();
        let until = Instant::now() + Duration::from_secs(5);
        while !(app.isActive() && window.isKeyWindow() && window.isMainWindow())
            && Instant::now() < until
        {
            pump_once(&NSRunLoop::currentRunLoop(), None);
            pump_work_application_event(&app, &mut application_events, input)?;
        }
        window
    } else {
        new_window(mtm)?
    };
    let view = window.contentView().ok_or("actor_host_view")?;
    let parent = RawWindowHandle::AppKit(AppKitWindowHandle::new(
        NonNull::from(&*view).cast::<c_void>(),
    ));
    let data = tempfile::tempdir().map_err(|_| "actor_temp_profile")?;
    let (sender, receiver) = std::sync::mpsc::sync_channel::<Box<dyn FnOnce() + Send>>(256);
    let dispatch: crate::MainThreadDispatch =
        Arc::new(move |operation| sender.try_send(operation).is_ok());
    let fatal = Arc::new(std::sync::Mutex::new(None));
    let fatal_sink = fatal.clone();
    let policy = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let policy_sink = policy.clone();
    let generation =
        zephium_core::blocker::ContentPolicyGeneration::new(1).ok_or("actor_policy_generation")?;
    let engine = Arc::new(
        crate::install(
            parent,
            dispatch,
            data.path().to_owned(),
            zephium_core::runtime_security::RuntimeSecurityAdvisories::new(),
            crate::InitialUserContent::new(
                zephium_core::ports::engine::UserContentGeneration::new(1)
                    .ok_or("actor_generation")?,
                Default::default(),
            ),
            move |event| {
                if let crate::EngineEvent::ContentRulesSettled {
                    profile: settled,
                    requested,
                    settlement,
                } = &event
                {
                    if *settled == profile && *requested == generation {
                        policy_sink.store(
                            if matches!(
                                settlement,
                                zephium_core::ports::engine::ContentRuleSettlement::Applied { .. }
                            ) {
                                1
                            } else {
                                2
                            },
                            Ordering::Release,
                        );
                    }
                }
                if let Some(events) = &events {
                    events(event);
                }
            },
            move |reason| {
                *fatal_sink
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
            },
        )
        .map_err(|_| "actor_engine_install")?,
    );
    let active = app.isActive();
    let main = window.isMainWindow();
    let run_loop = NSRunLoop::currentRunLoop();
    let result = (|| {
        if !application_policy {
            let probe_rules = PROBE_CONTENT_RULES.get().cloned();
            let compiling = if probe_rules.is_some() { 60 } else { 5 };
            if engine.install_content_rules(
                profile,
                generation,
                probe_rules.unwrap_or_else(|| {
                    zephium_core::blocker::ContentRules::allow_all(
                        zephium_core::blocker::ContentRuleDigest::from_bytes([0; 32]),
                    )
                }),
            ) != zephium_core::ports::engine::NativeDispatch::Scheduled
            {
                return Err("actor_profile_policy_dispatch");
            }
            let policy_deadline = Instant::now() + Duration::from_secs(compiling);
            while policy.load(Ordering::Acquire) == 0 && Instant::now() < policy_deadline {
                for _ in 0..256 {
                    let Ok(operation) = receiver.try_recv() else {
                        break;
                    };
                    run_work_operation(operation);
                }
                pump_once(&run_loop, None);
            }
            if policy.load(Ordering::Acquire) != 1 {
                return Err("actor_profile_policy");
            }
        }
        let mut poll = start(engine.clone())?;
        let deadline = Instant::now() + timeout;
        loop {
            if foreground {
                pump_work_application_event(&app, &mut application_events, input)?;
            }
            for _ in 0..256 {
                let Ok(operation) = receiver.try_recv() else {
                    break;
                };
                run_work_operation(operation);
            }
            let failure = *fatal
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let failure = failure.or_else(|| {
                // Hidden port probes must never acquire foreground. The full
                // application host permits normal focus changes; each shipping
                // observation independently enforces exact foreground ownership.
                (!foreground
                    && (window.isVisible()
                        || window.isKeyWindow()
                        || window.isMainWindow() != main
                        || app.isActive() != active))
                    .then_some("actor_focus_isolation")
            });
            if let Some(result) = poll(failure.is_some()) {
                return failure.map_or(result, Err);
            }
            if Instant::now() >= deadline {
                return Err("actor_host_deadline");
            }
            pump_once(&run_loop, None);
        }
    })();
    if !teardown.host_required(result.is_ok()) {
        // The full application's success callback requires ShellShutdown
        // Clean, which already consumed this exact engine barrier and joined
        // its workers. Reissuing that one-shot barrier is a protocol error.
        window.close();
        return result;
    }
    let shutdown = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let callback = shutdown.clone();
    engine.shutdown(Box::new(move |clean| {
        callback.store(if clean { 1 } else { 2 }, Ordering::Release);
    }));
    let deadline = Instant::now() + TEARDOWN_TIMEOUT;
    while shutdown.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
        if foreground {
            let _ = pump_work_application_event(&app, &mut application_events, input);
        }
        for _ in 0..256 {
            let Ok(operation) = receiver.try_recv() else {
                break;
            };
            run_work_operation(operation);
        }
        pump_once(&run_loop, None);
    }
    window.close();
    result?;
    if shutdown.load(Ordering::Acquire) != 1 {
        return Err("actor_host_teardown");
    }
    Ok(())
}

fn pump_work_application_event(
    app: &NSApplication,
    count: &mut u32,
    input: crate::MacosWorkProbeInput,
) -> Result<(), &'static str> {
    // NSRunLoop does not deliver NSApplication's queued lifecycle events.
    // Dequeue all event types so AppKit updates occlusion; filter input before delivery.
    if *count >= 8192 {
        return Err("actor_application_event_capacity");
    }
    objc2::rc::autoreleasepool(|_| {
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            objc2_app_kit::NSEventMask::Any,
            None,
            objc2_foundation::ns_string!("NSDefaultRunLoopMode"),
            true,
        ) {
            use objc2_app_kit::NSEventType;
            // A person's pointer moves constantly; only other events count
            // toward the flood cap.
            if !matches!(
                event.r#type(),
                NSEventType::MouseMoved
                    | NSEventType::MouseEntered
                    | NSEventType::MouseExited
                    | NSEventType::CursorUpdate
                    | NSEventType::LeftMouseDragged
                    | NSEventType::RightMouseDragged
                    | NSEventType::OtherMouseDragged
                    | NSEventType::ScrollWheel
            ) {
                *count += 1;
            }
            if !work_probe_delivers_event(input, event.r#type()) {
                return;
            }
            let input_kind = match event.r#type() {
                NSEventType::LeftMouseDown
                | NSEventType::RightMouseDown
                | NSEventType::OtherMouseDown => Some("PointerDown"),
                NSEventType::KeyDown => Some("KeyDown"),
                NSEventType::ScrollWheel => Some("Scroll"),
                _ => None,
            };
            if let Some(kind) = input_kind {
                eprintln!("work_probe input={kind} application_event={count}");
            }
            app.sendEvent(&event);
        }
    });
    Ok(())
}

fn work_probe_delivers_event(
    input: crate::MacosWorkProbeInput,
    event: objc2_app_kit::NSEventType,
) -> bool {
    use objc2_app_kit::NSEventType;
    match input {
        crate::MacosWorkProbeInput::LifecycleOnly => event == NSEventType::AppKitDefined,
        crate::MacosWorkProbeInput::Human => true,
    }
}

#[cfg(test)]
#[test]
fn only_explicit_human_probe_delivers_physical_input() {
    use objc2_app_kit::NSEventType;
    for event in [
        NSEventType::LeftMouseDown,
        NSEventType::LeftMouseUp,
        NSEventType::RightMouseDown,
        NSEventType::RightMouseUp,
        NSEventType::OtherMouseDown,
        NSEventType::OtherMouseUp,
        NSEventType::MouseMoved,
        NSEventType::LeftMouseDragged,
        NSEventType::RightMouseDragged,
        NSEventType::KeyDown,
        NSEventType::KeyUp,
        NSEventType::FlagsChanged,
        NSEventType::ScrollWheel,
        NSEventType::TabletPoint,
        NSEventType::TabletProximity,
        NSEventType::Gesture,
        NSEventType::Magnify,
        NSEventType::Swipe,
        NSEventType::Rotate,
        NSEventType::BeginGesture,
        NSEventType::EndGesture,
        NSEventType::DirectTouch,
        NSEventType::Pressure,
        NSEventType::QuickLook,
        NSEventType::SystemDefined,
        NSEventType::ApplicationDefined,
        NSEventType::Periodic,
        NSEventType::CursorUpdate,
        NSEventType(63),
    ] {
        assert!(!work_probe_delivers_event(
            crate::MacosWorkProbeInput::LifecycleOnly,
            event
        ));
        assert!(work_probe_delivers_event(
            crate::MacosWorkProbeInput::Human,
            event
        ));
    }
    assert!(work_probe_delivers_event(
        crate::MacosWorkProbeInput::LifecycleOnly,
        NSEventType::AppKitDefined
    ));
}

fn run_work_operation(operation: Box<dyn FnOnce() + Send>) {
    // Match Cocoa event dispatch: native construction/destruction may return
    // autoreleased configuration/data-store references. A pool only around
    // runUntilDate does not drain references created outside that pool.
    objc2::rc::autoreleasepool(|_| operation());
}

#[cfg(test)]
#[test]
fn work_host_drains_native_dispatch_autoreleases_in_every_phase() {
    let source = include_str!("agentic_semantic_probe.rs");
    let host = source
        .split("fn run_work_host(")
        .nth(1)
        .unwrap()
        .split("fn run_work_operation(")
        .next()
        .unwrap();
    assert_eq!(host.matches("run_work_operation(operation)").count(), 3);
    assert!(!host.contains("operation();"));
}

fn pump_once(run_loop: &NSRunLoop, native_guard: Option<&ProbeNativeState<'_>>) {
    if let Some(native_guard) = native_guard {
        native_guard.sample();
    }
    objc2::rc::autoreleasepool(|_| {
        run_loop.runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(
            RUN_LOOP_SLICE.as_secs_f64(),
        ));
    });
    if let Some(native_guard) = native_guard {
        native_guard.sample();
    }
}
