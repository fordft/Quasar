//! Fixed, bounded loopback fixtures for release-excluded native probes.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::probe_recipes::{backend_name, case_name, MAX_NATIVE_INPUT_RUNTIME_ROW};
use crate::{FixtureCase, InputBackend};

const MAX_REQUEST_BYTES: usize = 4 * 1_024;
// No form, submission controls, scripts, event handlers, frames, remote assets
// or persistence. The initial value deliberately differs from the trusted goal.
const RETAINED_LOCAL_FORM_HTML: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'none'; form-action 'none'; frame-src 'none'; base-uri 'none'"><title>Quasar local preparation witness</title><body><main><h1>Local draft</h1><label for="draft">Draft</label><input id="draft" type="text" value="Unprepared" autocomplete="off"><p>This field has no submission or persistence.</p></main></body></html>"#;
const RETAINED_BACK_ORIGIN_HTML: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'none'; form-action 'none'; frame-src 'none'; base-uri 'none'"><title>Quasar release card</title><body><main><h1>Quasar Release Card</h1><p>Release code</p><h2>ZEPH-R7-492</h2><p>Verify this candidate against the relevant linked record before reporting from this card.</p><nav aria-label="Release references"><a href="/retained-back/register-v1.html">Compatibility register</a><a href="/retained-back/handbook-v1.html">Release handbook</a></nav></main></body></html>"#;
const RETAINED_BACK_REGISTER_HTML: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'none'; form-action 'none'; frame-src 'none'; base-uri 'none'"><title>Quasar compatibility register</title><body><main><h1>Compatibility Register</h1><h2>ZEPH-R7-492</h2><p>Status: cleared for native history validation.</p><p>The authoritative release code remains on the release card.</p></main></body></html>"#;
const RETAINED_BACK_HANDBOOK_HTML: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'none'; form-action 'none'; frame-src 'none'; base-uri 'none'"><title>Quasar release handbook</title><body><main><h1>Release Handbook</h1><p>This handbook describes process only and does not contain compatibility status.</p></main></body></html>"#;
// A maximum 112-row matrix reloads the top document for activation isolation;
// each load may fetch the fixed child frame and one favicon.
const MAX_REQUESTS: usize = 512;
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const SEMANTIC_MUTATION_GATE_TIMEOUT: Duration = Duration::from_secs(15);
const SEMANTIC_MUTATION_TRIGGER_PATH: &str = "/semantic-runtime-mutation-trigger-v1.js";
const SEMANTIC_LOCATION_GATE_TIMEOUT: Duration = Duration::from_secs(15);
const SEMANTIC_LOCATION_TRIGGER_PATH: &str = "/semantic-location-trigger-v1.js";
const SEMANTIC_LOCATION_REPLACED_PATH: &str = "/semantic-location-replaced-v1.html";

#[derive(Clone, Copy)]
#[repr(u8)]
enum FixtureWorkerFailure {
    NonLoopbackPeer = 1,
    RequestBudget = 2,
    SemanticMutation = 3,
    SemanticLocation = 4,
    Accept = 5,
    Panicked = 6,
}

impl FixtureWorkerFailure {
    fn record(self, failure: &AtomicU8) {
        let _ = failure.compare_exchange(0, self as u8, Ordering::AcqRel, Ordering::Acquire);
    }
}

enum FixtureRequestError {
    Connection,
    SemanticMutation,
    SemanticLocation,
}

impl From<std::io::Error> for FixtureRequestError {
    fn from(_: std::io::Error) -> Self {
        Self::Connection
    }
}

struct SemanticGateError;

/// Closed fixture routes. Arbitrary files and caller-supplied responses are impossible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureRoute {
    /// Native-input matrix fixture.
    NativeInput,
    /// Deterministic hostile-page boundary fixture.
    HostilePage,
    /// Same-origin frame used by the native-input fixture.
    SameOriginFrame,
    /// First fixed document for production semantic-runtime qualification.
    SemanticRuntime,
    /// Provider-free hidden-view rendering readiness diagnostic.
    SemanticRendering,
    /// Fixed local browser/OS surface qualification after native text entry.
    OwnedSurfaceProbe,
    /// Script-free, non-submitting local field for retained action qualification.
    RetainedLocalForm,
    /// Original document for the retained native-Back qualification.
    RetainedBackOrigin,
    /// Relevant linked record for the retained native-Back qualification.
    RetainedBackRegister,
    /// Irrelevant observed-link alternative for open-route qualification.
    RetainedBackHandbook,
    /// Controlled-input document for the release-excluded page-world relay proof.
    SemanticRuntimeRelay,
    /// Real-WebKit adversarial page-world relay qualification document.
    SemanticRuntimeRelayHostile,
    /// Replacement document proving semantic world-epoch rotation.
    SemanticRuntimeReplacement,
    /// Bounded same-origin context pressure proving native event ceilings.
    SemanticRuntimeEventFlood,
    /// Same-document mutation and stale-anchor qualification fixture.
    SemanticRuntimeMutation,
    /// Same-document History API replacement qualification fixture.
    SemanticLocationMutation,
    /// First hop of the fixed same-origin semantic redirect chain.
    SemanticRedirectStart,
    /// Intermediate hop of the fixed same-origin semantic redirect chain.
    SemanticRedirectHop,
    /// Final document of the fixed same-origin semantic redirect chain.
    SemanticRedirectFinal,
    /// First half of the fixed redirect loop used to prove hop refusal.
    SemanticRedirectLoopA,
    /// Second half of the fixed redirect loop used to prove hop refusal.
    SemanticRedirectLoopB,
}

impl FixtureRoute {
    fn path(self) -> &'static str {
        match self {
            Self::NativeInput => "/native-input-v1.html",
            Self::HostilePage => "/hostile-v1.html",
            Self::SameOriginFrame => "/frame-v1.html",
            Self::SemanticRuntime => "/semantic-runtime-v1.html",
            Self::SemanticRendering => "/semantic-rendering-v1.html",
            Self::OwnedSurfaceProbe => "/owned-surface-probe-v1.html",
            Self::RetainedLocalForm => "/retained-local-form-v1.html",
            Self::RetainedBackOrigin => "/retained-back/origin-v1.html",
            Self::RetainedBackRegister => "/retained-back/register-v1.html",
            Self::RetainedBackHandbook => "/retained-back/handbook-v1.html",
            Self::SemanticRuntimeRelay => "/semantic-runtime-relay-v1.html",
            Self::SemanticRuntimeRelayHostile => "/semantic-runtime-relay-hostile-v1.html",
            Self::SemanticRuntimeReplacement => "/semantic-runtime-replacement-v1.html",
            Self::SemanticRuntimeEventFlood => "/semantic-runtime-event-flood-v1.html",
            Self::SemanticRuntimeMutation => "/semantic-runtime-mutation-v1.html",
            Self::SemanticLocationMutation => "/semantic-location-mutation-v1.html",
            Self::SemanticRedirectStart => "/semantic-redirect-start-v1",
            Self::SemanticRedirectHop => "/semantic-redirect-hop-v1",
            Self::SemanticRedirectFinal => "/semantic-redirect-final-v1.html",
            Self::SemanticRedirectLoopA => "/semantic-redirect-loop-a-v1",
            Self::SemanticRedirectLoopB => "/semantic-redirect-loop-b-v1",
        }
    }
}

#[derive(Default)]
struct SemanticGateState {
    waiting: bool,
    released: bool,
    completed: bool,
}

#[derive(Default)]
struct SemanticGate {
    state: Mutex<SemanticGateState>,
    wake: Condvar,
}

impl SemanticGate {
    fn wait_for_release(
        &self,
        stop: &AtomicBool,
        timeout: Duration,
    ) -> Result<bool, SemanticGateError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(SemanticGateError)?;
        let mut state = self.state.lock().map_err(|_| SemanticGateError)?;
        if state.waiting || state.released || state.completed {
            return Err(SemanticGateError);
        }
        state.waiting = true;
        self.wake.notify_all();
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(false);
            }
            if state.released {
                return Ok(true);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(SemanticGateError);
            }
            let wait = deadline.saturating_duration_since(now).min(IO_TIMEOUT);
            let (next, _) = self
                .wake
                .wait_timeout(state, wait)
                .map_err(|_| SemanticGateError)?;
            state = next;
        }
    }

    fn release(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !state.waiting || state.released || state.completed {
            return false;
        }
        state.released = true;
        self.wake.notify_all();
        true
    }

    fn mark_completed(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !state.waiting || !state.released || state.completed {
            return false;
        }
        state.completed = true;
        self.wake.notify_all();
        true
    }

    fn waiting(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.waiting && !state.released && !state.completed)
    }

    fn completed(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.completed)
    }

    fn wake_for_stop(&self) {
        self.wake.notify_all();
    }
}

/// Fixed loopback server with one thread, a finite request budget, and no logs.
pub struct FixtureServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    failure: Arc<AtomicU8>,
    semantic_mutation: Arc<SemanticGate>,
    semantic_location: Arc<SemanticGate>,
    thread: Option<JoinHandle<()>>,
}

impl FixtureServer {
    /// Starts a server bound only to an ephemeral IPv4 loopback port.
    pub fn start() -> Result<Self, FixtureServerError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        if !address.ip().is_loopback() {
            return Err(FixtureServerError::NonLoopbackBind);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(AtomicU8::new(0));
        let semantic_mutation = Arc::new(SemanticGate::default());
        let semantic_location = Arc::new(SemanticGate::default());
        let worker_stop = Arc::clone(&stop);
        let worker_failure = Arc::clone(&failure);
        let worker_semantic_mutation = Arc::clone(&semantic_mutation);
        let worker_semantic_location = Arc::clone(&semantic_location);
        let thread = thread::Builder::new()
            .name("zephium-agentic-fixture".to_owned())
            .spawn(move || {
                serve(
                    listener,
                    &worker_stop,
                    &worker_failure,
                    &worker_semantic_mutation,
                    &worker_semantic_location,
                );
            })?;
        Ok(Self {
            address,
            stop,
            failure,
            semantic_mutation,
            semantic_location,
            thread: Some(thread),
        })
    }

    /// Returns the fixed loopback URL for one closed route.
    pub fn url(&self, route: FixtureRoute) -> String {
        format!("http://127.0.0.1:{}{}", self.address.port(), route.path())
    }

    /// Returns one closed, correlation-bearing native-input fixture URL.
    ///
    /// The fixed server accepts this exact query shape only. Values are
    /// derived from closed Rust enums and are never caller-provided strings.
    pub fn native_input_url(
        &self,
        row: u16,
        case: FixtureCase,
        backend: InputBackend,
    ) -> Option<String> {
        if row == 0 || row > MAX_NATIVE_INPUT_RUNTIME_ROW {
            return None;
        }
        Some(format!(
            "{}?row={row}&case={}&backend={}",
            self.url(FixtureRoute::NativeInput),
            case_name(case),
            backend_name(backend),
        ))
    }

    /// Returns false after a listener, invariant, worker, or request-budget failure.
    pub fn is_healthy(&self) -> bool {
        self.failure.load(Ordering::Acquire) == 0
    }

    /// True only while the one fixed mutation-trigger response is blocked.
    pub fn semantic_mutation_waiting(&self) -> bool {
        self.semantic_mutation.waiting()
    }

    /// Releases the one fixed mutation trigger exactly once.
    pub fn release_semantic_mutation(&self) -> bool {
        self.semantic_mutation.release()
    }

    /// True only after the released trigger response was written completely.
    pub fn semantic_mutation_completed(&self) -> bool {
        self.semantic_mutation.completed()
    }

    /// Returns the only allowed same-document replacement target.
    pub fn semantic_location_replacement_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}{SEMANTIC_LOCATION_REPLACED_PATH}",
            self.address.port()
        )
    }

    /// True only while the one fixed History API trigger response is blocked.
    pub fn semantic_location_waiting(&self) -> bool {
        self.semantic_location.waiting()
    }

    /// Releases the one fixed History API trigger exactly once.
    pub fn release_semantic_location(&self) -> bool {
        self.semantic_location.release()
    }

    /// True only after the released History API trigger response was written completely.
    pub fn semantic_location_completed(&self) -> bool {
        self.semantic_location.completed()
    }

    /// Stops the listener, joins its sole worker, and reports worker failure.
    pub fn shutdown(mut self) -> Result<(), FixtureServerError> {
        self.stop_and_join();
        match self.failure.load(Ordering::Acquire) {
            0 => Ok(()),
            value if value == FixtureWorkerFailure::NonLoopbackPeer as u8 => {
                Err(FixtureServerError::NonLoopbackPeer)
            }
            value if value == FixtureWorkerFailure::RequestBudget as u8 => {
                Err(FixtureServerError::RequestBudgetExhausted)
            }
            value if value == FixtureWorkerFailure::SemanticMutation as u8 => {
                Err(FixtureServerError::SemanticMutationInvariant)
            }
            value if value == FixtureWorkerFailure::SemanticLocation as u8 => {
                Err(FixtureServerError::SemanticLocationInvariant)
            }
            value if value == FixtureWorkerFailure::Accept as u8 => {
                Err(FixtureServerError::AcceptFailed)
            }
            value if value == FixtureWorkerFailure::Panicked as u8 => {
                Err(FixtureServerError::WorkerPanicked)
            }
            _ => Err(FixtureServerError::WorkerFailed),
        }
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.semantic_mutation.wake_for_stop();
        self.semantic_location.wake_for_stop();
        // Wake the blocking accept promptly. No bytes are sent and this
        // connection remains loopback-only.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(50));
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                FixtureWorkerFailure::Panicked.record(&self.failure);
            }
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn serve(
    listener: TcpListener,
    stop: &AtomicBool,
    failure: &AtomicU8,
    semantic_mutation: &SemanticGate,
    semantic_location: &SemanticGate,
) {
    let mut served = 0_usize;
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, peer)) => {
                if stop.load(Ordering::Acquire) {
                    return;
                }
                if !peer.ip().is_loopback() {
                    FixtureWorkerFailure::NonLoopbackPeer.record(failure);
                    continue;
                }
                served = match served.checked_add(1) {
                    Some(value) if value <= MAX_REQUESTS => value,
                    _ => {
                        FixtureWorkerFailure::RequestBudget.record(failure);
                        return;
                    }
                };
                // WebKit may close speculative or favicon sockets without a
                // complete request. Each required response is independently
                // proven by the caller, while every connection remains under
                // the fixed byte/time/request ceilings, so a peer close is not
                // a server invariant failure.
                match handle(stream, stop, semantic_mutation, semantic_location) {
                    Err(FixtureRequestError::SemanticMutation) => {
                        FixtureWorkerFailure::SemanticMutation.record(failure);
                    }
                    Err(FixtureRequestError::SemanticLocation) => {
                        FixtureWorkerFailure::SemanticLocation.record(failure);
                    }
                    Ok(()) | Err(FixtureRequestError::Connection) => {}
                }
            }
            Err(_) => {
                if stop.load(Ordering::Acquire) {
                    return;
                }
                FixtureWorkerFailure::Accept.record(failure);
                return;
            }
        }
    }
}

fn handle(
    mut stream: TcpStream,
    stop: &AtomicBool,
    semantic_mutation: &SemanticGate,
    semantic_location: &SemanticGate,
) -> Result<(), FixtureRequestError> {
    // Keep this explicit if the listener implementation changes: each
    // connection uses blocking I/O under a hard deadline.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut request = [0_u8; MAX_REQUEST_BYTES];
    let mut received = 0_usize;
    while received < request.len() {
        let count = stream.read(&mut request[received..])?;
        if count == 0 {
            return Ok(());
        }
        received += count;
        if request[..received]
            .windows(4)
            .any(|window| window == b"\r\n\r\n")
        {
            break;
        }
    }
    if !request[..received]
        .windows(4)
        .any(|window| window == b"\r\n\r\n")
    {
        write_response(&mut stream, 413, "text/plain; charset=utf-8", b"bounded")?;
        return Ok(());
    }
    let first_line_end = request[..received]
        .windows(2)
        .position(|window| window == b"\r\n")
        .unwrap_or(received);
    let first_line = &request[..first_line_end];
    if is_fixed_get_request(first_line, SEMANTIC_MUTATION_TRIGGER_PATH) {
        if !semantic_mutation
            .wait_for_release(stop, SEMANTIC_MUTATION_GATE_TIMEOUT)
            .map_err(|_| FixtureRequestError::SemanticMutation)?
        {
            return Ok(());
        }
        write_response(
            &mut stream,
            200,
            "application/javascript; charset=utf-8",
            SEMANTIC_RUNTIME_MUTATION_TRIGGER_SCRIPT.as_bytes(),
        )?;
        if !semantic_mutation.mark_completed() {
            return Err(FixtureRequestError::SemanticMutation);
        }
        return Ok(());
    }
    if is_fixed_get_request(first_line, SEMANTIC_LOCATION_TRIGGER_PATH) {
        if !semantic_location
            .wait_for_release(stop, SEMANTIC_LOCATION_GATE_TIMEOUT)
            .map_err(|_| FixtureRequestError::SemanticLocation)?
        {
            return Ok(());
        }
        write_response(
            &mut stream,
            200,
            "application/javascript; charset=utf-8",
            SEMANTIC_LOCATION_TRIGGER_SCRIPT.as_bytes(),
        )?;
        if !semantic_location.mark_completed() {
            return Err(FixtureRequestError::SemanticLocation);
        }
        return Ok(());
    }
    let redirect = if is_fixed_get_request(first_line, FixtureRoute::SemanticRedirectStart.path()) {
        Some(FixtureRoute::SemanticRedirectHop)
    } else if is_fixed_get_request(first_line, FixtureRoute::SemanticRedirectHop.path()) {
        Some(FixtureRoute::SemanticRedirectFinal)
    } else if is_fixed_get_request(first_line, FixtureRoute::SemanticRedirectLoopA.path()) {
        Some(FixtureRoute::SemanticRedirectLoopB)
    } else if is_fixed_get_request(first_line, FixtureRoute::SemanticRedirectLoopB.path()) {
        Some(FixtureRoute::SemanticRedirectLoopA)
    } else {
        None
    };
    if let Some(location) = redirect {
        write_redirect(&mut stream, location)?;
        return Ok(());
    }
    let (status, content_type, body, script_policy) = match first_line {
        b"GET /owned-surface-probe-v1.html HTTP/1.1"
        | b"GET /owned-surface-probe-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            include_bytes!("../assets/owned-surface-probe-v1.html").as_slice(),
            FixtureScriptPolicy::InlineOnly,
        ),
        line if is_native_input_request(line) => (
            200,
            "text/html; charset=utf-8",
            NATIVE_INPUT_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /hostile-v1.html HTTP/1.1" | b"GET /hostile-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            HOSTILE_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /frame-v1.html HTTP/1.1" | b"GET /frame-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            FRAME_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /semantic-runtime-v1.html HTTP/1.1" | b"GET /semantic-runtime-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RUNTIME_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /semantic-rendering-v1.html HTTP/1.1"
        | b"GET /semantic-rendering-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RENDERING_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /retained-local-form-v1.html HTTP/1.1"
        | b"GET /retained-local-form-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            RETAINED_LOCAL_FORM_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /retained-back/origin-v1.html HTTP/1.1"
        | b"GET /retained-back/origin-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            RETAINED_BACK_ORIGIN_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /retained-back/register-v1.html HTTP/1.1"
        | b"GET /retained-back/register-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            RETAINED_BACK_REGISTER_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /retained-back/handbook-v1.html HTTP/1.1"
        | b"GET /retained-back/handbook-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            RETAINED_BACK_HANDBOOK_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /semantic-runtime-relay-v1.html HTTP/1.1"
        | b"GET /semantic-runtime-relay-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RUNTIME_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /semantic-runtime-relay-hostile-v1.html HTTP/1.1"
        | b"GET /semantic-runtime-relay-hostile-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RUNTIME_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /semantic-runtime-replacement-v1.html HTTP/1.1"
        | b"GET /semantic-runtime-replacement-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RUNTIME_REPLACEMENT_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /semantic-runtime-event-flood-v1.html HTTP/1.1"
        | b"GET /semantic-runtime-event-flood-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RUNTIME_EVENT_FLOOD_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /semantic-runtime-mutation-v1.html HTTP/1.1"
        | b"GET /semantic-runtime-mutation-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RUNTIME_MUTATION_HTML.as_bytes(),
            FixtureScriptPolicy::SameOrigin,
        ),
        b"GET /semantic-location-mutation-v1.html HTTP/1.1"
        | b"GET /semantic-location-mutation-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_LOCATION_MUTATION_HTML.as_bytes(),
            FixtureScriptPolicy::SameOrigin,
        ),
        b"GET /semantic-redirect-final-v1.html HTTP/1.1"
        | b"GET /semantic-redirect-final-v1.html HTTP/1.0" => (
            200,
            "text/html; charset=utf-8",
            SEMANTIC_RUNTIME_HTML.as_bytes(),
            FixtureScriptPolicy::InlineOnly,
        ),
        b"GET /favicon.ico HTTP/1.1" | b"GET /favicon.ico HTTP/1.0" => (
            204,
            "image/x-icon",
            &[] as &[u8],
            FixtureScriptPolicy::InlineOnly,
        ),
        _ => (
            404,
            "text/plain; charset=utf-8",
            b"not found" as &[u8],
            FixtureScriptPolicy::InlineOnly,
        ),
    };
    write_response_with_policy(&mut stream, status, content_type, body, script_policy)?;
    Ok(())
}

fn is_fixed_get_request(line: &[u8], expected_target: &str) -> bool {
    let Ok(line) = std::str::from_utf8(line) else {
        return false;
    };
    let mut fields = line.split(' ');
    matches!(
        (fields.next(), fields.next(), fields.next(), fields.next()),
        (Some("GET"), Some(target), Some("HTTP/1.0" | "HTTP/1.1"), None)
            if target == expected_target
    )
}

fn is_native_input_request(line: &[u8]) -> bool {
    let Ok(line) = std::str::from_utf8(line) else {
        return false;
    };
    let mut fields = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    if method != "GET" || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return false;
    }
    if target == FixtureRoute::NativeInput.path() {
        return true;
    }
    let Some(query) = target.strip_prefix("/native-input-v1.html?") else {
        return false;
    };
    let mut parts = query.split('&');
    let (Some(row), Some(case), Some(backend), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Some(row) = row.strip_prefix("row=") else {
        return false;
    };
    if row.is_empty()
        || row.len() > 3
        || !row.bytes().all(|byte| byte.is_ascii_digit())
        || row.starts_with('0')
        || row
            .parse::<u16>()
            .map_or(true, |row| row > MAX_NATIVE_INPUT_RUNTIME_ROW)
    {
        return false;
    }
    let Some(case) = case.strip_prefix("case=") else {
        return false;
    };
    let Some(backend) = backend.strip_prefix("backend=") else {
        return false;
    };
    const CASES: [FixtureCase; 14] = [
        FixtureCase::Button,
        FixtureCase::Link,
        FixtureCase::TextInput,
        FixtureCase::ContentEditable,
        FixtureCase::Select,
        FixtureCase::PointerMouse,
        FixtureCase::Keyboard,
        FixtureCase::TransientActivation,
        FixtureCase::Popup,
        FixtureCase::ClipboardGate,
        FixtureCase::Drag,
        FixtureCase::Iframe,
        FixtureCase::OpenShadow,
        FixtureCase::ClosedShadow,
    ];
    const BACKENDS: [InputBackend; 8] = [
        InputBackend::FixedDomRecipe,
        InputBackend::MacosAppKitEvent,
        InputBackend::MacosAccessibility,
        InputBackend::MacosFocusedOsInput,
        InputBackend::WindowsHwndInput,
        InputBackend::WindowsCompositionInput,
        InputBackend::WindowsCdpInput,
        InputBackend::HumanBaseline,
    ];
    CASES.into_iter().any(|value| case_name(value) == case)
        && BACKENDS
            .into_iter()
            .any(|value| backend_name(value) == backend)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), std::io::Error> {
    write_response_with_policy(
        stream,
        status,
        content_type,
        body,
        FixtureScriptPolicy::InlineOnly,
    )
}

fn write_redirect(stream: &mut TcpStream, destination: FixtureRoute) -> Result<(), std::io::Error> {
    let location = destination.path();
    let header = format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\n\r\n"
    );
    stream.write_all(header.as_bytes())?;
    stream.flush()
}

#[derive(Clone, Copy)]
enum FixtureScriptPolicy {
    InlineOnly,
    SameOrigin,
}

fn write_response_with_policy(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    script_policy: FixtureScriptPolicy,
) -> Result<(), std::io::Error> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let script_source = match script_policy {
        FixtureScriptPolicy::InlineOnly => "'unsafe-inline'",
        FixtureScriptPolicy::SameOrigin => "'self' 'unsafe-inline'",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src {script_source}; style-src 'unsafe-inline'; frame-src 'self'; connect-src 'none'; img-src 'none'; object-src 'none'; base-uri 'none'; form-action 'self'\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// Fixture-server construction or terminal worker failure.
#[derive(Debug, Error)]
pub enum FixtureServerError {
    /// Loopback listener or worker thread could not be created.
    #[error("fixture server I/O failed")]
    Io(#[from] std::io::Error),
    /// Listener unexpectedly resolved to a non-loopback address.
    #[error("fixture server was not bound to loopback")]
    NonLoopbackBind,
    /// Worker reported an unclassified invariant failure.
    #[error("fixture server worker failed")]
    WorkerFailed,
    /// Worker accepted a peer that was not loopback.
    #[error("fixture server rejected a non-loopback peer")]
    NonLoopbackPeer,
    /// Worker exhausted its fixed connection budget.
    #[error("fixture server request budget was exhausted")]
    RequestBudgetExhausted,
    /// The exact single-use semantic mutation gate drifted.
    #[error("fixture server semantic mutation invariant failed")]
    SemanticMutationInvariant,
    /// The exact single-use History API gate drifted.
    #[error("fixture server semantic location invariant failed")]
    SemanticLocationInvariant,
    /// The loopback listener failed while accepting a connection.
    #[error("fixture server accept failed")]
    AcceptFailed,
    /// The sole fixture worker panicked.
    #[error("fixture server worker panicked")]
    WorkerPanicked,
}

const NATIVE_INPUT_HTML: &str = r###"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Quasar native input fixture v1</title>
  <style>
    body { font: 16px system-ui; margin: 24px; }
    .grid { display: grid; grid-template-columns: repeat(2, minmax(180px, 1fr)); gap: 16px; }
    button, input, select, [contenteditable], a, .drop { min-height: 38px; padding: 8px; border: 1px solid #555; }
    .drop { min-height: 70px; }
    iframe { width: 100%; height: 100px; border: 1px solid #555; }
  </style>
</head>
<body>
  <main class="grid">
    <button id="button" type="button">Button</button>
    <a id="link" href="#linked">Link</a>
    <input id="text-input" type="text" value="fixture">
    <div id="content-editable" contenteditable="true">fixture</div>
    <select id="select"><option>one</option><option>two</option></select>
    <button id="activation-button" type="button">Activation</button>
    <button id="popup-button" type="button">Popup</button>
    <button id="clipboard-button" type="button">Clipboard gate</button>
    <div id="drag-source" draggable="true">Drag source</div>
    <div id="drop-target" class="drop">Drop target</div>
    <iframe id="frame" src="/frame-v1.html" title="same-origin frame"></iframe>
    <div id="open-shadow-host"></div>
    <div id="closed-shadow-host" tabindex="0" role="button" aria-label="Closed shadow control"></div>
  </main>
  <script>
  (() => {
    'use strict';
    const MAX_EVENTS = 64;
    const ids = Object.freeze({
      'button': 'button', 'link': 'link', 'text-input': 'text_input',
      'content-editable': 'content_editable', 'select': 'select',
      'activation-button': 'activation_button', 'popup-button': 'popup_button',
      'clipboard-button': 'clipboard_button', 'drag-source': 'drag_source',
      'drop-target': 'drop_target', 'open-shadow-button': 'open_shadow_button',
      'open-shadow-host': 'open_shadow_button', 'closed-shadow-host': 'closed_shadow_host'
    });
    const eventKinds = new Set([
      'focus', 'blur', 'pointerenter', 'pointermove', 'pointerdown', 'pointerup',
      'mouseenter', 'mousemove', 'mousedown', 'mouseup', 'click', 'keydown',
      'beforeinput', 'input', 'keyup', 'change', 'dragstart', 'dragenter',
      'dragover', 'drop', 'dragend'
    ]);
    let state;
    const emptyState = (caseName) => ({
      case: caseName, events: [], actualTarget: null, targetVerified: false,
      activeBefore: navigator.userActivation ? navigator.userActivation.isActive : false,
      activeDuringEvent: false, activeAfterEvent: false, activeAfterSettle: false,
      hasBeenActive: navigator.userActivation ? navigator.userActivation.hasBeenActive : false,
      activationCaptureScheduled: false,
      navigationObserved: false, popupObserved: false, clipboardGate: 'not_applicable',
      buttonCount: 0, inputLength: 0, contentLength: 0, selectedIndex: 0,
      dropObserved: false
    });
    state = emptyState('none');

    const openRoot = document.getElementById('open-shadow-host').attachShadow({mode: 'open'});
    const openButton = document.createElement('button');
    openButton.id = 'open-shadow-button';
    openButton.type = 'button';
    openButton.textContent = 'Open shadow control';
    openRoot.append(openButton);
    const closedRoot = document.getElementById('closed-shadow-host').attachShadow({mode: 'closed'});
    const closedButton = document.createElement('button');
    closedButton.type = 'button';
    closedButton.textContent = 'Closed shadow internal';
    closedRoot.append(closedButton);

    const targetName = (event) => {
      const path = typeof event.composedPath === 'function' ? event.composedPath() : [event.target];
      for (const node of path) {
        if (node && node.id && ids[node.id]) return ids[node.id];
      }
      return 'document';
    };
    const scheduleActivationCapture = () => {
      if (state.activationCaptureScheduled) return;
      state.activationCaptureScheduled = true;
      queueMicrotask(() => {
        state.activeAfterEvent = !!(navigator.userActivation && navigator.userActivation.isActive);
        state.hasBeenActive = state.hasBeenActive ||
          !!(navigator.userActivation && navigator.userActivation.hasBeenActive);
      });
      setTimeout(() => {
        state.activeAfterSettle = !!(navigator.userActivation && navigator.userActivation.isActive);
        state.hasBeenActive = state.hasBeenActive ||
          !!(navigator.userActivation && navigator.userActivation.hasBeenActive);
      }, 50);
    };
    const record = (event) => {
      if (!eventKinds.has(event.type) || state.events.length >= MAX_EVENTS) return;
      const target = targetName(event);
      state.actualTarget = target;
      state.activeDuringEvent = state.activeDuringEvent ||
        !!(navigator.userActivation && navigator.userActivation.isActive);
      state.events.push({kind: event.type.replace('pointerenter', 'pointer_enter')
        .replace('pointermove', 'pointer_move').replace('pointerdown', 'pointer_down')
        .replace('pointerup', 'pointer_up').replace('mouseenter', 'mouse_enter')
        .replace('mousemove', 'mouse_move').replace('mousedown', 'mouse_down')
        .replace('mouseup', 'mouse_up').replace('beforeinput', 'before_input')
        .replace('keydown', 'key_down').replace('keyup', 'key_up')
        .replace('dragstart', 'drag_start').replace('dragenter', 'drag_enter')
        .replace('dragover', 'drag_over').replace('dragend', 'drag_end'),
        is_trusted: event.isTrusted, target});
      if (event.type === 'click' || event.type === 'keydown') scheduleActivationCapture();
    };
    for (const kind of eventKinds) document.addEventListener(kind, record, true);

    document.getElementById('button').addEventListener('click', () => { state.buttonCount += 1; });
    document.getElementById('link').addEventListener('click', () => {
      queueMicrotask(() => { state.navigationObserved = location.hash === '#linked'; });
    });
    document.getElementById('text-input').addEventListener('input', (event) => {
      state.inputLength = event.currentTarget.value.length;
    });
    document.getElementById('content-editable').addEventListener('input', (event) => {
      state.contentLength = event.currentTarget.textContent.length;
    });
    document.getElementById('select').addEventListener('change', (event) => {
      state.selectedIndex = event.currentTarget.selectedIndex;
    });
    document.getElementById('popup-button').addEventListener('click', () => {
      const popup = window.open('about:blank', 'zephium-probe-popup', 'popup,width=120,height=80');
      state.popupObserved = !!popup;
      if (popup) setTimeout(() => popup.close(), 0);
    });
    document.getElementById('clipboard-button').addEventListener('click', () => {
      state.clipboardGate = navigator.clipboard && isSecureContext ? 'indeterminate' : 'denied';
    });
    document.getElementById('drag-source').addEventListener('dragstart', (event) => {
      if (event.dataTransfer) event.dataTransfer.setData('text/plain', 'fixed-probe-token');
    });
    document.getElementById('drop-target').addEventListener('dragover', (event) => event.preventDefault());
    document.getElementById('drop-target').addEventListener('drop', (event) => {
      event.preventDefault(); state.dropObserved = true;
    });
    window.addEventListener('message', (event) => {
      if (event.origin !== location.origin || !event.data || event.data.fixture !== 'frame-v1') return;
      if (state.events.length < MAX_EVENTS && event.data.kind === 'click') {
        state.events.push({kind: 'click', is_trusted: event.data.isTrusted === true, target: 'frame_button'});
        state.actualTarget = 'frame_button';
        state.activeDuringEvent = state.activeDuringEvent || event.data.activeDuringEvent === true;
        state.hasBeenActive = state.hasBeenActive || event.data.hasBeenActive === true;
        scheduleActivationCapture();
      }
    });

    const verify = () => {
      const expected = ({button: 'button', link: 'link', text_input: 'text_input',
        content_editable: 'content_editable', select: 'select', pointer_mouse: 'button',
        keyboard: 'text_input', transient_activation: 'activation_button', popup: 'popup_button',
        clipboard_gate: 'clipboard_button', drag: 'drop_target', iframe: 'frame_button',
        open_shadow: 'open_shadow_button', closed_shadow: 'closed_shadow_host'})[state.case];
      const effect = ({button: state.buttonCount > 0, link: state.navigationObserved,
        text_input: state.inputLength > 7, content_editable: state.contentLength > 7,
        select: state.selectedIndex === 1, pointer_mouse: state.events.some(e => e.kind === 'click'),
        keyboard: state.events.some(e => e.kind === 'key_down'),
        transient_activation: state.events.some(e => e.kind === 'click'),
        popup: state.events.some(e => e.kind === 'click'),
        clipboard_gate: state.clipboardGate !== 'not_applicable', drag: state.dropObserved,
        iframe: state.actualTarget === 'frame_button', open_shadow: state.actualTarget === 'open_shadow_button',
        closed_shadow: state.actualTarget === 'closed_shadow_host'})[state.case] === true;
      const intendedTargetObserved = state.events.some(event => event.target === expected);
      if (intendedTargetObserved) state.actualTarget = expected;
      state.targetVerified = intendedTargetObserved && effect;
      state.hasBeenActive = !!(navigator.userActivation && navigator.userActivation.hasBeenActive);
    };
    const api = Object.freeze({
      reset(caseName) {
        if (typeof caseName !== 'string' || caseName.length > 32) return false;
        state = emptyState(caseName); location.hash = '';
        document.getElementById('text-input').value = 'fixture';
        document.getElementById('content-editable').textContent = 'fixture';
        document.getElementById('select').selectedIndex = 0;
        return true;
      },
      readJson() {
        verify();
        return JSON.stringify(state);
      }
    });
    Object.defineProperty(window, '__zephiumNativeInputFixtureV1', {
      value: api, configurable: false, enumerable: false, writable: false
    });

    const query = new URLSearchParams(location.search);
    const row = query.get('row');
    const caseName = query.get('case');
    const backend = query.get('backend');
    const allowedCases = new Set([
      'button', 'link', 'text_input', 'content_editable', 'select',
      'pointer_mouse', 'keyboard', 'transient_activation', 'popup',
      'clipboard_gate', 'drag', 'iframe', 'open_shadow', 'closed_shadow'
    ]);
    const allowedBackends = new Set([
      'fixed_dom_recipe', 'macos_app_kit_event', 'macos_accessibility',
      'macos_focused_os_input', 'windows_hwnd_input',
      'windows_composition_input', 'windows_cdp_input', 'human_baseline'
    ]);
    if (row && /^[1-9][0-9]{0,2}$/.test(row) && Number(row) <= 128 &&
        caseName && allowedCases.has(caseName) && backend && allowedBackends.has(backend) &&
        [...query.keys()].length === 3 && api.reset(caseName)) {
      document.documentElement.dataset.probeRow = row;
      document.documentElement.dataset.probeCase = caseName;
      document.documentElement.dataset.probeBackend = backend;
      new MutationObserver(() => {
        if (document.documentElement.dataset.probeReadRequest !== row) return;
        document.documentElement.dataset.probeEvidence = api.readJson();
        document.documentElement.dataset.probeEvidenceRow = row;
      }).observe(document.documentElement, {
        attributes: true, attributeFilter: ['data-probe-read-request']
      });
    }
    document.documentElement.dataset.fixtureReady = 'v1';
  })();
  </script>
</body>
</html>"###;

const FRAME_HTML: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Frame v1</title></head>
<body><button id="frame-button" type="button">Frame button</button>
<script>
document.getElementById('frame-button').addEventListener('click', (event) => {
  parent.postMessage({fixture: 'frame-v1', kind: 'click', isTrusted: event.isTrusted,
    activeDuringEvent: !!(navigator.userActivation && navigator.userActivation.isActive),
    hasBeenActive: !!(navigator.userActivation && navigator.userActivation.hasBeenActive)},
    location.origin);
});
document.documentElement.dataset.fixtureReady = 'frame-v1';
</script></body></html>"###;

const SEMANTIC_RENDERING_HTML: &str = r###"<!doctype html>
<meta charset="utf-8">
<title>Rendering readiness fixture</title>
<style>body { margin: 16px; font: 16px sans-serif; } p { margin: 8px; }</style>
<h1>Rendering readiness fixture</h1>
<p id="document">Document loading</p><p id="load">Load pending</p>
<div id="microtask" hidden><p>Microtask ready</p></div>
<div id="timer" hidden><p>Timer ready</p></div>
<div id="animation" hidden><p>Animation frame ready</p></div>
<script>
(() => {
  'use strict';
  const state = document.getElementById('document');
  const update = () => { state.textContent = 'Document ' + document.readyState; };
  document.addEventListener('readystatechange', update);
  update();
  if (window.location.hash === '#windows-retained-progress') {
    const progress = document.createElement('h3');
    let ticks = 0;
    progress.textContent = 'Retained native progress 0';
    document.body.append(progress);
    const timer = setInterval(() => {
      ticks += 1;
      progress.textContent = 'Retained native progress ' + ticks;
      if (ticks === 4096) clearInterval(timer);
    }, 500);
  }
  window.addEventListener('load', () => { document.getElementById('load').textContent = 'Load ready'; }, { once: true });
  Promise.resolve().then(() => { document.getElementById('microtask').hidden = false; });
  setTimeout(() => { document.getElementById('timer').hidden = false; }, 0);
  requestAnimationFrame(() => { document.getElementById('animation').hidden = false; });
})();
</script>"###;

const SEMANTIC_RUNTIME_HTML: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'self'">
<meta name="referrer" content="no-referrer">
<title>Semantic runtime fixture v1</title></head>
<body>
<main aria-label="First semantic epoch">
  <h1>First semantic epoch</h1>
  <div id="semantic-fill-editable" contenteditable="true" role="textbox" aria-label="Semantic fill editable">fixture editable</div>
  <span role="heading" aria-level="3" id="semantic-fill-editable-status">Semantic fill editable pending</span>
  <aside aria-label="Fixed Fill diagnostic fixtures" style="position:absolute;right:0;top:0;width:220px;font-size:10px;line-height:12px">
  <div role="textbox" aria-label="Fill support missing attribute">diagnostic</div>
  <label contenteditable="true" role="textbox" aria-label="Fill support unsupported tag">diagnostic</label>
  <div id="nested-editor" contenteditable="true" role="group"><div id="nested-leaf" contenteditable="true" role="textbox" aria-label="Fill support editable ancestor">diagnostic</div><span id="nested-sibling" contenteditable="false"><b>retained sibling</b></span></div>
  <div id="nested-destination" contenteditable="true" role="group"></div>
  <span role="heading" aria-level="3" id="nested-status">Nested editor pending</span>
  <div contenteditable="true" role="group"><div contenteditable="true" role="textbox" aria-label="Fill support rich editable ancestor"><span>diagnostic</span></div></div>
  <div contenteditable="true" role="textbox" aria-label="Fill support element child"><span>diagnostic</span></div>
  <div contenteditable="true" role="textbox" aria-label="Fill support other child">diagnostic<!-- retained structural marker --></div>
  <div contenteditable="true" role="textbox" aria-label="Fill support readonly" aria-readonly="true">diagnostic</div>
  <div contenteditable="true" role="textbox" aria-label="Fill support disabled" aria-disabled="true">diagnostic</div>
  <input type="email" role="textbox" aria-label="Fill support unsupported control" value="diagnostic">
  <div id="fill-support-child-limit" contenteditable="true" role="textbox" aria-label="Fill support child limit"></div>
  <script>for (let i = 0; i < 129; i++) document.getElementById('fill-support-child-limit').appendChild(document.createTextNode('x'));</script>
  </aside>
  <p id="bridge-status">Page bridge unresolved</p>
  <details><summary id="primary-semantic-action"><span>Primary semantic action</span></summary><p>Native disclosure content</p></details>
  <p id="primary-action-activation" aria-label="Primary action activation pending"></p>
  <p id="primary-action-settled" aria-label="Primary action settle pending"></p>
  <input id="semantic-fill-text" type="text" aria-label="Semantic fill text" value="fixture text">
  <h3 id="semantic-fill-text-status">Semantic fill text pending</h3>
  <input id="semantic-fill-search" type="search" aria-label="Semantic fill search" value="fixture search">
  <h3 id="semantic-fill-search-status">Semantic fill search pending</h3>
  <textarea id="semantic-fill-textarea" aria-label="Semantic fill textarea">fixture textarea</textarea>
  <h3 id="semantic-fill-textarea-status">Semantic fill textarea pending</h3>
  <input id="semantic-hostile-fill" type="text" aria-label="Semantic hostile fill" value="hostile-before">
  <input id="semantic-hostile-recovery" type="text" aria-label="Semantic hostile recovery" value="recovery-before">
  <h3 id="semantic-hostile-recovery-status">Semantic hostile recovery pending</h3>
  <h3 id="semantic-hostile-status">Semantic hostile relay pending</h3>
  <div id="semantic-hostile-quarantine"></div>
  <input id="semantic-hostile-credential-fill" type="text" aria-label="Semantic hostile credential fill" value="credential-before">
  <span id="semantic-hostile-credential-label" hidden>API key</span>
  <h3 id="semantic-hostile-credential-status">Semantic hostile credential pending</h3>
  <h3 id="semantic-hostile-credential-recovery-status">Semantic hostile credential recovery pending</h3>
  <input type="password" aria-label="Password field" value="fixture-password-value">
  <input type="text" aria-label="Token field" value="Bearer abcdefghijklmnop">
  <div id="open-shadow-host"></div>
  <div id="closed-shadow-host" aria-label="Closed shadow boundary"></div>
  <iframe src="/frame-v1.html" title="Semantic child frame"></iframe>
</main>
<script>
(() => {
  'use strict';
  const primaryAction = document.getElementById('primary-semantic-action');
  const activation = document.getElementById('primary-action-activation');
  const settled = document.getElementById('primary-action-settled');
  primaryAction.addEventListener('click', (event) => {
    const userActivation = navigator.userActivation;
    const trusted = event.isTrusted;
    const activeDuring = !!userActivation?.isActive;
    const stickyDuring = !!userActivation?.hasBeenActive;
    const popup = window.open('/semantic-popup-denied-v1.html', '_blank');
    const popupDenied = popup === null;
    primaryAction.setAttribute(
      'aria-label',
      event.isTrusted ? 'Primary semantic action applied trusted' : 'Primary semantic action applied untrusted'
    );
    activation.setAttribute(
      'aria-label',
      `Primary action activation during ${userActivation?.isActive ? 'active' : 'inactive'} sticky ${userActivation?.hasBeenActive ? 'active' : 'inactive'} popup ${popup === null ? 'denied' : 'admitted'}`
    );
    setTimeout(() => {
      const activeAfterSettle = !!userActivation?.isActive;
      const stickyAfterSettle = !!userActivation?.hasBeenActive;
      settled.setAttribute(
        'aria-label',
        `Primary action settle ${activeAfterSettle ? 'active' : 'inactive'} sticky ${stickyAfterSettle ? 'active' : 'inactive'}`
      );
      primaryAction.setAttribute(
        'aria-label',
        `Primary semantic action applied ${trusted ? 'trusted' : 'untrusted'} activation during ${activeDuring ? 'active' : 'inactive'} sticky ${stickyDuring ? 'active' : 'inactive'} popup ${popupDenied ? 'denied' : 'admitted'} settle ${activeAfterSettle ? 'active' : 'inactive'} sticky ${stickyAfterSettle ? 'active' : 'inactive'}`
      );
    }, 75);
  });
  const installFillProbe = (targetId, statusId, expected) => {
    const target = document.getElementById(targetId);
    const status = document.getElementById(statusId);
    const editable = target.isContentEditable;
    const valueGetter = Object.getOwnPropertyDescriptor(
      editable ? Node.prototype : target instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype,
      editable ? 'textContent' : 'value'
    ).get;
    const counts = {beforeinput: 0, input: 0, change: 0};
    const countClass = (count) => count === 0 ? '0' : count === 1 ? '1' : 'many';
    const publishCounts = (eventClass) => {
      status.textContent =
        `Semantic fill events ${eventClass} before ${countClass(counts.beforeinput)} input ${countClass(counts.input)} change ${countClass(counts.change)}`;
    };
    target.addEventListener('beforeinput', () => {
      counts.beforeinput += 1;
      publishCounts('beforeinput observed');
    });
    target.addEventListener('change', () => {
      counts.change += 1;
      publishCounts('change observed');
    });
    target.addEventListener('input', (event) => {
      counts.input += 1;
      publishCounts('input observed');
      const activation = navigator.userActivation;
      const trusted = event.isTrusted;
      const replacement = event.inputType === 'insertReplacementText';
      const exactData = event.data === expected;
      const exactProjection = Reflect.apply(valueGetter, target, []) === expected;
      const activeDuring = !!activation?.isActive;
      const stickyDuring = !!activation?.hasBeenActive;
      const popup = window.open('/semantic-popup-denied-v1.html', '_blank');
      const popupDenied = popup === null;
      if (popup) setTimeout(() => popup.close(), 0);
      setTimeout(() => {
        const activeAfterSettle = !!activation?.isActive;
        const stickyAfterSettle = !!activation?.hasBeenActive;
        status.textContent =
          `Semantic fill observed input ${trusted ? 'trusted' : 'untrusted'} replacement ${replacement ? 'yes' : 'no'} data ${exactData ? 'exact' : 'mismatch'} projection ${exactProjection ? 'exact' : 'mismatch'} before ${countClass(counts.beforeinput)} input ${countClass(counts.input)} change ${countClass(counts.change)} popup ${popupDenied ? 'denied' : 'admitted'} activation during ${activeDuring ? 'active' : 'inactive'} sticky ${stickyDuring ? 'active' : 'inactive'} settle ${activeAfterSettle ? 'active' : 'inactive'} sticky ${stickyAfterSettle ? 'active' : 'inactive'}`;
      }, 75);
    });
  };
  installFillProbe('semantic-fill-text', 'semantic-fill-text-status', 'Quasar fixed text');
  installFillProbe('semantic-fill-search', 'semantic-fill-search-status', 'Quasar fixed search');
  installFillProbe('semantic-fill-editable', 'semantic-fill-editable-status', '  Quasar fixed editable\nline two  ');
  installFillProbe(
    'semantic-fill-textarea',
    'semantic-fill-textarea-status',
    '  Quasar  fixed textarea\nline two  '
  );
  installFillProbe(
    'semantic-hostile-recovery',
    'semantic-hostile-recovery-status',
    'Quasar hostile recovery'
  );
  // Provider-free Windows native-input guard fixture. The beforeinput handler
  // changes the admitted identity and editing selection before browser mutation.
  if (window.location.hash === '#windows-rich-guard') {
    const target = document.createElement('div');
    target.contentEditable = 'true'; target.setAttribute('role', 'textbox');
    target.setAttribute('aria-label', 'Windows guarded rich editor');
    target.innerHTML = '<p>guard original</p>';
    const decoy = document.createElement('div');
    decoy.contentEditable = 'true'; decoy.setAttribute('role', 'textbox');
    decoy.setAttribute('aria-label', 'Windows unapproved rich editor');
    decoy.innerHTML = '<p>decoy original</p>';
    const quarantine = document.createElement('section');
    const witness = document.createElement('h3');
    witness.textContent = 'Windows rich guard pending';
    const panel = document.createElement('section');
    panel.style.cssText = 'position:absolute;right:240px;top:0;width:220px;font-size:10px;line-height:12px;z-index:1;background:white';
    panel.append(target, decoy, quarantine, witness);
    document.body.append(panel);
    let before = 0, inputs = 0;
    const publish = trusted => {
      witness.textContent = `Windows rich guard before ${before === 1 ? 'one' : 'other'} input ${inputs === 0 ? 'zero' : 'other'} target ${target.textContent === 'guard original' ? 'intact' : 'changed'} decoy ${decoy.textContent === 'decoy original' ? 'intact' : 'changed'} trusted ${trusted ? 'yes' : 'no'}`;
    };
    for (const editor of [target, decoy]) editor.addEventListener('input', event => { inputs += 1; publish(event.isTrusted); });
    target.addEventListener('beforeinput', event => {
      before += 1;
      target.setAttribute('aria-label', 'Windows changed rich editor');
      quarantine.append(target);
      decoy.focus({preventScroll: true});
      const range = document.createRange(); range.selectNodeContents(decoy);
      const selection = document.getSelection(); selection.removeAllRanges(); selection.addRange(range);
      setTimeout(() => publish(event.isTrusted), 0);
    }, {once: true});
    // Positive counterpart uses the same paragraph/model contract as the
    // controlled Slack/Linear/Notion replicas, including their empty editor.
    const controlled = document.createElement('div');
    controlled.contentEditable = 'true'; controlled.setAttribute('role', 'textbox');
    controlled.setAttribute('aria-label', 'Windows controlled rich editor');
    controlled.innerHTML = '<p><br></p>';
    const controlledWitness = document.createElement('h3');
    controlledWitness.textContent = 'Windows controlled rich pending';
    panel.append(controlled, controlledWitness);
    let controlledModel = '', controlledBefore = 0, controlledInputs = 0;
    let controlledTrusted = false;
    const paragraphs = () => {
      const lines = [];
      for (const node of controlled.childNodes) {
        if (node.nodeType !== 1 || node.tagName !== 'P') return null;
        lines.push(node.textContent.replace(/\u00a0/g, ' '));
      }
      return lines.length ? lines.join('\n') : '';
    };
    const renderControlled = () => {
      controlled.innerHTML = '';
      const paragraph = document.createElement('p');
      if (controlledModel) paragraph.textContent = controlledModel;
      else paragraph.appendChild(document.createElement('br'));
      controlled.appendChild(paragraph);
    };
    controlled.addEventListener('beforeinput', event => {
      controlledBefore += 1; controlledTrusted = event.isTrusted;
    });
    controlled.addEventListener('input', event => {
      controlledInputs += 1; controlledTrusted = controlledTrusted && event.isTrusted;
      const model = paragraphs();
      if (model === null) renderControlled();
      else controlledModel = model;
      controlledWitness.textContent = `Windows controlled rich model ${controlledModel === 'On my way, ten minutes out' ? 'exact' : 'mismatch'} paragraphs ${paragraphs() !== null ? 'retained' : 'lost'} trusted ${controlledTrusted ? 'yes' : 'no'} before ${controlledBefore === 1 ? 'one' : 'other'} input ${controlledInputs === 1 ? 'one' : 'other'}`;
    });
    new MutationObserver(() => { if (paragraphs() === null) renderControlled(); })
      .observe(controlled, {childList: true, subtree: true, characterData: true});
  }
  // Local fixture application: delegated editor handler owns a tiny model and
  // rerenders only the admitted leaf. The recipe must never edit this parent,
  // its noneditable formatted sibling, or a newly selected editing context.
  const editor = document.getElementById('nested-editor');
  const leaf = document.getElementById('nested-leaf');
  const sibling = document.getElementById('nested-sibling');
  const siblingChild = sibling.firstChild;
  const destination = document.getElementById('nested-destination');
  const nestedStatus = document.getElementById('nested-status');
  const textAccess = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent');
  const text = node => Reflect.apply(textAccess.get, node, []);
  const replace = (node, value) => Reflect.apply(textAccess.set, node, [value]);
  let nestedModel = 'diagnostic';
  let nestedInputs = 0;
  const axProbe = window.location.hash === '#native-ax-fill';
  // Release-excluded trusted interaction proof. This application owns an
  // editing unit and may reconstruct its text leaf after native editing. The
  // native probe never obtains a selector or program from these attributes.
  const trustedCase = window.location.hash.slice('#native-unit-'.length);
  if (window.location.hash.startsWith('#native-unit-') &&
      ['normal', 'retarget', 'retarget-before-native', 'retarget-text-input', 'cancel', 'navigation', 'takeover-before', 'takeover-after'].includes(trustedCase)) {
    const unit = document.createElement('div');
    unit.contentEditable = 'true';
    unit.setAttribute('role', 'textbox');
    unit.setAttribute('aria-label', 'Trusted editing unit');
    const original = document.createElement('span');
    original.contentEditable = 'true';
    original.setAttribute('aria-label', 'Trusted unit text');
    original.textContent = 'unit original';
    const preserved = document.createElement('b');
    preserved.contentEditable = 'false';
    preserved.textContent = 'unit sibling';
    unit.append(original, preserved);
    const decoy = document.createElement('div');
    decoy.contentEditable = 'true';
    decoy.setAttribute('role', 'textbox');
    decoy.setAttribute('aria-label', 'Trusted unit decoy');
    decoy.textContent = 'unit decoy';
    const setup = document.createElement('h3');
    const report = document.createElement('h3');
    // Keep both independently verified fields in the viewport even when the
    // takeover-before row deliberately performs no native autoscroll.
    primaryAction.before(unit, decoy, setup, report);
    let inputs = 0, trusted = 0, before = 0, active = false, sticky = false;
    let model = 'unit original', popup = false, rerendered = false;
    const select = node => {
      node.focus({preventScroll: true});
      const range = document.createRange(); range.selectNodeContents(node);
      const selection = document.getSelection();
      selection.removeAllRanges(); selection.addRange(range);
    };
    const value = () => Array.from(unit.childNodes).filter(n => n !== preserved).map(text).join('');
    const render = () => {
      // The application reconciles by logical unit, replacing site-owned leaf
      // identity without modifying the formatted noneditable sibling.
      for (const node of Array.from(unit.childNodes)) if (node !== preserved) node.remove();
      const next = document.createElement('span'); next.contentEditable = 'true';
      next.setAttribute('aria-label', 'Trusted unit text');
      next.textContent = model; unit.insertBefore(next, preserved); rerendered = true;
    };
    const publish = () => {
      const siblingOK = preserved.parentNode === unit && text(preserved) === 'unit sibling';
      report.textContent = `Trusted unit witness inputs ${inputs} trusted ${trusted} before ${before}` +
        ` model ${model === 'unit replacement' ? 'accepted' : model === 'unit original' ? 'original' : 'other'}` +
        ` rerender ${rerendered ? 'yes' : 'no'} leaf ${original.isConnected ? 'same' : 'replaced'}` +
        ` sibling ${siblingOK ? 'intact' : 'changed'} decoy ${text(decoy) === 'unit decoy' ? 'intact' : 'changed'}` +
        ` active ${active ? 'yes' : 'no'} sticky ${sticky ? 'yes' : 'no'} popup ${popup ? 'admitted' : 'denied'}` +
        ` retained ${value() === model ? 'yes' : 'no'}`;
    };
    unit.addEventListener('beforeinput', event => {
      before += 1;
      if (trustedCase === 'cancel') event.preventDefault();
      if (trustedCase === 'retarget') select(decoy);
      if (trustedCase === 'navigation') window.location.assign('https://native-unit-denied.invalid/');
    });
    unit.addEventListener('textInput', () => {
      if (trustedCase === 'retarget-text-input') select(decoy);
    });
    document.addEventListener('input', event => {
      if (event.target !== unit && event.target !== decoy && event.target !== original) return;
      inputs += 1; trusted += event.isTrusted ? 1 : 0;
      active ||= navigator.userActivation.isActive;
      sticky ||= navigator.userActivation.hasBeenActive;
      popup ||= window.open('about:blank', '_blank') !== null;
      if (event.target !== decoy && event.isTrusted) {
        model = value(); queueMicrotask(render);
      }
    }, true);
    primaryAction.addEventListener('click', () => {
      select(original);
      const selected = document.getSelection();
      const range = selected.rangeCount === 1 ? selected.getRangeAt(0) : null;
      setup.textContent = range && range.startContainer === original && range.endContainer === original
        ? 'Trusted unit setup exact logical range' : 'Trusted unit setup ambiguous';
      if (trustedCase === 'retarget-before-native') setTimeout(() => select(decoy), 450);
      // Fixed bounded observation schedule, not a model-selected wait/command.
      setTimeout(publish, 1200);
    }, {once: true});
  }
  // Fixed responder authority experiment. The page owns setup; the native
  // probe invokes only the existing ref-bound semantic button, never eval.
  const responderCase = window.location.hash.slice('#native-responder-'.length);
  if (window.location.hash.startsWith('#native-responder-') &&
      ['flat', 'nested', 'retarget', 'cross-leaf'].includes(responderCase)) {
    const flat = document.getElementById('semantic-fill-editable');
    const target = responderCase === 'flat' ? flat : leaf;
    const decoy = document.createElement('div');
    decoy.contentEditable = 'true';
    decoy.setAttribute('role', 'textbox');
    decoy.setAttribute('aria-label', 'Responder unapproved decoy');
    decoy.textContent = text(target);
    document.body.appendChild(decoy);
    const setup = document.createElement('h3');
    const focus = document.createElement('h3');
    const challenge = document.createElement('h3');
    const witness = document.createElement('h3');
    document.body.append(setup, focus, challenge, witness);
    let edits = 0;
    for (const kind of ['beforeinput', 'input', 'change', 'compositionstart', 'compositionupdate', 'compositionend']) {
      document.addEventListener(kind, () => { edits += 1; }, true);
    }
    const select = node => {
      node.focus({preventScroll: true});
      const range = document.createRange();
      range.selectNodeContents(node);
      const selection = document.getSelection();
      selection.removeAllRanges(); selection.addRange(range);
    };
    primaryAction.addEventListener('click', () => {
      select(target);
      const selected = document.getSelection();
      setup.textContent = selected.rangeCount === 1 &&
        selected.getRangeAt(0).startContainer === target &&
        selected.getRangeAt(0).endContainer === target
        ? 'Responder setup selected exact leaf' : 'Responder setup ambiguous';
      focus.textContent = document.activeElement === target ? 'Responder setup focus exact leaf'
        : document.activeElement === editor ? 'Responder setup focus editing ancestor'
        : 'Responder setup focus other';
      setTimeout(() => {
        if (responderCase === 'retarget') {
          select(decoy);
          challenge.textContent = document.activeElement === decoy &&
            document.getSelection().getRangeAt(0).startContainer === decoy
            ? 'Responder challenge focus decoy selection decoy' : 'Responder challenge failed';
        } else if (responderCase === 'cross-leaf') {
          const range = document.createRange();
          range.setStart(leaf.firstChild, 0);
          range.setEnd(sibling.firstChild.firstChild, text(sibling).length);
          const selection = document.getSelection();
          selection.removeAllRanges(); selection.addRange(range);
          const actual = selection.rangeCount === 1 ? selection.getRangeAt(0) : null;
          challenge.textContent = actual && actual.startContainer === leaf.firstChild &&
            actual.endContainer === sibling.firstChild.firstChild
            ? 'Responder challenge selection crosses leaf sibling'
            : 'Responder challenge selection normalized by engine';
        } else {
          const selection = document.getSelection();
          const actual = selection.rangeCount === 1 ? selection.getRangeAt(0) : null;
          challenge.textContent = actual && actual.startContainer === target && actual.endContainer === target
            ? 'Responder challenge selection remains exact leaf' : 'Responder challenge failed';
        }
        witness.textContent = intact() && text(flat) === 'fixture editable' &&
          text(leaf) === 'diagnostic' && text(decoy) === text(target) &&
          nestedModel === 'diagnostic' && nestedInputs === 0 && edits === 0
          ? 'Responder witness fields intact events zero model unchanged' : 'Responder witness mismatch';
      }, 450);
    }, {once: true});
  }
  if (axProbe) {
    const flat = document.getElementById('semantic-fill-editable');
    let flatModel = text(flat);
    flat.addEventListener('input', () => {
      flatModel = text(flat);
      queueMicrotask(() => {
        replace(flat, flatModel);
        const report = document.createElement('h3');
        report.textContent = flatModel === 'ax flat replacement' && !navigator.userActivation.isActive && !navigator.userActivation.hasBeenActive
          ? 'AX flat application retained' : 'AX flat application mismatch';
        document.body.appendChild(report);
      });
    });
  }
  const intact = () => leaf.parentNode === editor && sibling.parentNode === editor &&
    sibling.firstChild === siblingChild && text(sibling) === 'retained sibling';
  editor.addEventListener('input', event => {
    if (event.target !== leaf) return;
    nestedInputs += 1;
    nestedModel = text(leaf);
    queueMicrotask(() => {
      replace(leaf, nestedModel);
      if (axProbe) {
        nestedStatus.textContent = intact() && nestedModel === 'nested replacement' && nestedInputs === 1 &&
          !navigator.userActivation.isActive && !navigator.userActivation.hasBeenActive
          ? 'AX nested application retained' : 'AX nested application mismatch';
        return;
      }
      nestedStatus.textContent = intact() && text(leaf) === 'nested replacement' &&
        nestedModel === 'nested replacement' && nestedInputs === 1 &&
        event.data === nestedModel && event.inputType === 'insertReplacementText' && !event.isTrusted
        ? 'Nested editor delegated model retained' : 'Nested editor model mismatch';
    });
  });
  leaf.addEventListener('beforeinput', event => {
    const mode = event.data;
    if (!['nested move', 'nested relabel', 'nested protected', 'nested credential', 'nested editability', 'nested rich'].includes(mode)) return;
    if (mode === 'nested move') destination.appendChild(leaf);
    if (mode === 'nested relabel') leaf.setAttribute('aria-label', 'Repurposed nested leaf');
    if (mode === 'nested protected') editor.setAttribute('aria-readonly', 'true');
    if (mode === 'nested credential') editor.setAttribute('aria-label', 'Password');
    if (mode === 'nested editability') editor.setAttribute('contenteditable', 'false');
    if (mode === 'nested rich') leaf.appendChild(document.createElement('span'));
    setTimeout(() => {
      const unchanged = text(leaf) === 'nested replacement' && nestedModel === 'nested replacement' && nestedInputs === 1;
      editor.insertBefore(leaf, sibling);
      leaf.setAttribute('aria-label', 'Fill support editable ancestor');
      editor.removeAttribute('aria-readonly'); editor.removeAttribute('aria-label');
      editor.setAttribute('contenteditable', 'true');
      if (mode === 'nested rich') leaf.lastChild.remove();
      nestedStatus.textContent = unchanged && intact()
        ? `Nested editor refused ${mode} intact` : 'Nested editor unauthorized mutation';
    }, 75);
  });
  if (window.location.pathname === '/semantic-runtime-relay-hostile-v1.html') {
    const unauthorized = document.createElement('input');
    unauthorized.type = 'text';
    unauthorized.hidden = true;
    unauthorized.value = 'unapproved original';
    document.body.append(unauthorized);
    unauthorized.dispatchEvent(new CustomEvent('zephium-fill-request-v1', {
      detail: JSON.stringify({ v: 1, a: 997, z: 'unapproved replacement' }),
      bubbles: false, cancelable: false, composed: false
    }));
    const unauthorizedStatus = document.createElement('h3');
    unauthorizedStatus.textContent = unauthorized.value === 'unapproved original'
      ? 'Semantic page-origin fill blocked'
      : 'Semantic page-origin fill unauthorized';
    document.body.append(unauthorizedStatus);
    unauthorized.remove();
    // Deterministic negative control for the old transport: a reconciler
    // registered first removes COMMAND before the consumer can reread it.
    const batchingStatus = document.createElement('h3');
    batchingStatus.textContent = 'Semantic relay transport pending';
    document.body.append(batchingStatus);
    const legacy = document.createElement('input');
    legacy.hidden = true;
    document.body.append(legacy);
    const legacyCommand = 'data-zephium-fill-relay-command-v1';
    const legacyTerminal = 'data-zephium-fill-relay-terminal-v1';
    const relayGet = Element.prototype.getAttribute;
    const readRelayAttr = (node, name) => Reflect.apply(relayGet, node, [name]);
    const stripping = new MutationObserver(() => legacy.removeAttribute(legacyCommand));
    stripping.observe(legacy, { attributes: true, attributeFilter: [legacyCommand] });
    const consuming = new MutationObserver(() => {
      const lost = readRelayAttr(legacy, legacyCommand) === null &&
        readRelayAttr(legacy, legacyTerminal) === null;
      stripping.disconnect();
      consuming.disconnect();
      batchingStatus.textContent = lost
        ? 'Semantic relay transport legacy command-gone reproduced'
        : 'Semantic relay transport negative control failed';
      legacy.remove();
    });
    consuming.observe(legacy, { attributes: true, attributeFilter: [legacyCommand] });
    legacy.setAttribute(legacyCommand, 'closed-negative-control');

    // The actual admitted contenteditable action runs with the same hostile
    // reconciliation behavior. An attribute-based transport would be erased.
    const editable = document.getElementById('semantic-fill-editable');
    const interference = document.createElement('h3');
    interference.textContent = 'Semantic relay interference pending';
    document.body.append(interference);
    let transportAttributes = 0;
    const editableStripping = new MutationObserver((records) => {
      transportAttributes += records.length;
      editable.removeAttribute(legacyCommand);
      editable.removeAttribute(legacyTerminal);
    });
    editableStripping.observe(editable, {
      attributes: true, attributeFilter: [legacyCommand, legacyTerminal]
    });
    editable.addEventListener('input', () => {
      interference.textContent = transportAttributes === 0 &&
        readRelayAttr(editable, legacyCommand) === null && readRelayAttr(editable, legacyTerminal) === null
        ? 'Semantic relay attribute interference avoided'
        : 'Semantic relay attribute interference observed';
    });
    const fixedDispatch = EventTarget.prototype.dispatchEvent;
    const FixedCustomEvent = CustomEvent;
    const capturedValueGet = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').get;
    const capturedDetailGet = Object.getOwnPropertyDescriptor(CustomEvent.prototype, 'detail').get;
    const retarget = document.createElement('input');
    retarget.type = 'text';
    retarget.hidden = true;
    retarget.value = 'retarget original';
    document.body.append(retarget);
    const retargetStatus = document.createElement('h3');
    retargetStatus.textContent = 'Semantic captured-payload retarget pending';
    document.body.append(retargetStatus);
    retarget.addEventListener('input', () => {
      retargetStatus.textContent = 'Semantic captured-payload retarget unauthorized';
    });
    editable.addEventListener('zephium-fill-request-v1', event => {
      Reflect.apply(fixedDispatch, retarget, [new FixedCustomEvent('zephium-fill-request-v1', {
        detail: Reflect.apply(capturedDetailGet, event, [])
      })]);
    });
    editable.addEventListener('input', () => Promise.resolve().then(() => {
      retargetStatus.textContent = Reflect.apply(capturedValueGet, retarget, []) === 'retarget original'
        ? 'Semantic captured-payload retarget blocked'
        : 'Semantic captured-payload retarget unauthorized';
    }));
    const forgedReply = (node, detail) => Reflect.apply(fixedDispatch, node, [
      new FixedCustomEvent('zephium-fill-result-v1', { detail })
    ]);
    const hostile = document.getElementById('semantic-hostile-fill');
    const recovery = document.getElementById('semantic-hostile-recovery');
    const hostileStatus = document.getElementById('semantic-hostile-status');
    const quarantine = document.getElementById('semantic-hostile-quarantine');
    const credential = document.getElementById('semantic-hostile-credential-fill');
    const credentialStatus = document.getElementById('semantic-hostile-credential-status');
    const credentialRecoveryStatus = document.getElementById('semantic-hostile-credential-recovery-status');
    const main = hostile.parentNode;
    const hostileNextSibling = hostile.nextSibling;
    const relayTerminal = 'data-zephium-fill-relay-terminal-v1';
    const valueGetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').get;
    let armed = true;
    forgedReply(hostile, '1|5|ok');
    hostile.addEventListener('beforeinput', (event) => {
      if (!armed) return;
      armed = false;
      const activation = navigator.userActivation;
      const activeDuring = !!activation?.isActive;
      const stickyDuring = !!activation?.hasBeenActive;
      const popup = window.open('/semantic-popup-denied-v1.html', '_blank');
      const popupDenied = popup === null;
      if (popup) setTimeout(() => popup.close(), 0);
      forgedReply(hostile, '1|5|ok');
      forgedReply(recovery, '1|6|ok');
      quarantine.append(hostile);
      hostile.type = 'password';
      Promise.resolve().then(() => {
        hostile.type = 'text';
        main.insertBefore(hostile, hostileNextSibling);
        setTimeout(() => {
          const targetUnchanged = Reflect.apply(valueGetter, hostile, []) === 'hostile-before';
          const recoveryUnchanged = Reflect.apply(valueGetter, recovery, []) === 'recovery-before';
          hostileStatus.textContent =
            `Semantic hostile relay refused ${event.isTrusted ? 'trusted' : 'untrusted'} target ${targetUnchanged ? 'unchanged' : 'changed'} recovery ${recoveryUnchanged ? 'unchanged' : 'changed'} type ${hostile.type === 'text' ? 'restored' : 'changed'} target-marker ${hostile.hasAttribute(relayTerminal) ? 'present' : 'clear'} recovery-marker ${recovery.hasAttribute(relayTerminal) ? 'forged' : 'missing'} popup ${popupDenied ? 'denied' : 'admitted'} activation during ${activeDuring ? 'active' : 'inactive'} sticky ${stickyDuring ? 'active' : 'inactive'} settle ${activation?.isActive ? 'active' : 'inactive'} sticky ${activation?.hasBeenActive ? 'active' : 'inactive'}`;
        }, 75);
      });
    });
    recovery.addEventListener('input', () => {
      setTimeout(() => {
        hostileStatus.textContent =
          `Semantic hostile relay recovered target-marker ${hostile.hasAttribute(relayTerminal) ? 'present' : 'clear'} recovery-marker ${recovery.hasAttribute(relayTerminal) ? 'present' : 'clear'}`;
      }, 75);
    });
    let credentialArmed = true;
    credential.addEventListener('beforeinput', (event) => {
      if (!credentialArmed) return;
      credentialArmed = false;
      const activation = navigator.userActivation;
      const activeDuring = !!activation?.isActive;
      const stickyDuring = !!activation?.hasBeenActive;
      const popup = window.open('/semantic-popup-denied-v1.html', '_blank');
      const popupDenied = popup === null;
      if (popup) setTimeout(() => popup.close(), 0);
      forgedReply(credential, '1|7|ok');
      credential.setAttribute('aria-labelledby', 'semantic-hostile-credential-label');
      Promise.resolve().then(() => {
        credential.removeAttribute('aria-labelledby');
        setTimeout(() => {
          const unchanged = Reflect.apply(valueGetter, credential, []) === 'credential-before';
          credentialStatus.textContent =
            `Semantic hostile credential refused ${event.isTrusted ? 'trusted' : 'untrusted'} target ${unchanged ? 'unchanged' : 'changed'} credential-marker ${credential.hasAttribute('aria-labelledby') ? 'present' : 'clear'} target-marker ${credential.hasAttribute(relayTerminal) ? 'present' : 'clear'} popup ${popupDenied ? 'denied' : 'admitted'} activation during ${activeDuring ? 'active' : 'inactive'} sticky ${stickyDuring ? 'active' : 'inactive'} settle ${activation?.isActive ? 'active' : 'inactive'} sticky ${activation?.hasBeenActive ? 'active' : 'inactive'}`;
        }, 75);
      });
    });
    credential.addEventListener('input', (event) => {
      setTimeout(() => {
        const exact = Reflect.apply(valueGetter, credential, []) === 'Quasar credential recovery';
        credentialRecoveryStatus.textContent =
          `Semantic hostile credential recovered input ${event.isTrusted ? 'trusted' : 'untrusted'} value ${exact ? 'exact' : 'mismatch'} credential-marker ${credential.hasAttribute('aria-labelledby') ? 'present' : 'clear'} target-marker ${credential.hasAttribute(relayTerminal) ? 'present' : 'clear'}`;
      }, 75);
    });
  }
  const openRoot = document.getElementById('open-shadow-host').attachShadow({mode: 'open'});
  const openButton = document.createElement('button');
  openButton.type = 'button';
  openButton.setAttribute('aria-label', 'Open shadow semantic action');
  openRoot.append(openButton);
  const closedRoot = document.getElementById('closed-shadow-host').attachShadow({mode: 'closed'});
  const closedButton = document.createElement('button');
  closedButton.type = 'button';
  closedButton.setAttribute('aria-label', 'Closed internal must remain absent');
  closedRoot.append(closedButton);

  Object.defineProperty(window, '__zephiumSemanticRuntimeV1', {
    value: Object.freeze({invoke() { return 'page-world-forgery'; }}),
    configurable: false, enumerable: false, writable: false
  });
  let bridgeVisible = false;
  try {
    bridgeVisible = !!(window.webkit && window.webkit.messageHandlers &&
      window.webkit.messageHandlers.zephiumSemanticRuntimeV1);
  } catch (_) {
    bridgeVisible = true;
  }
  document.getElementById('bridge-status').textContent =
    bridgeVisible ? 'Page bridge present' : 'Page bridge absent';

  if (window.location.pathname !== '/semantic-runtime-relay-v1.html') {
    Object.defineProperty(Element.prototype, 'getAttribute', {
      value() { throw new Error('page-world prototype poison'); },
      configurable: true, writable: true
    });
    Object.defineProperty(HTMLInputElement.prototype, 'value', {
      get() { throw new Error('page-world input getter poison'); },
      set() { throw new Error('page-world input setter poison'); },
      configurable: true
    });
    Object.defineProperty(HTMLTextAreaElement.prototype, 'value', {
      get() { throw new Error('page-world textarea getter poison'); },
      set() { throw new Error('page-world textarea setter poison'); },
      configurable: true
    });
    Object.defineProperty(EventTarget.prototype, 'dispatchEvent', {
      value() { throw new Error('page-world event dispatch poison'); },
      configurable: true, writable: true
    });
    try {
      window.InputEvent = function PoisonedInputEvent() {
        throw new Error('page-world input event poison');
      };
    } catch (_) {
      // Some WebKit builds expose the constructor as non-writable.
    }
  }
  document.documentElement.dataset.fixtureReady = 'semantic-runtime-v1';
})();
</script>
</body></html>"###;

const SEMANTIC_RUNTIME_REPLACEMENT_HTML: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'unsafe-inline'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'self'">
<meta name="referrer" content="no-referrer">
<title>Semantic replacement fixture v1</title></head>
<body>
<main aria-label="Replacement semantic epoch">
  <h1>Replacement semantic epoch</h1>
  <p id="bridge-status">Page bridge unresolved</p>
  <button type="button" aria-label="Replacement semantic action">Run</button>
</main>
<script>
(() => {
  'use strict';
  Object.defineProperty(window, '__zephiumSemanticRuntimeV1', {
    value: Object.freeze({invoke() { return 'replacement-page-world-forgery'; }}),
    configurable: false, enumerable: false, writable: false
  });
  let bridgeVisible = false;
  try {
    bridgeVisible = !!(window.webkit && window.webkit.messageHandlers &&
      window.webkit.messageHandlers.zephiumSemanticRuntimeV1);
  } catch (_) {
    bridgeVisible = true;
  }
  document.getElementById('bridge-status').textContent =
    bridgeVisible ? 'Page bridge present' : 'Page bridge absent';
  document.documentElement.dataset.fixtureReady = 'semantic-runtime-replacement-v1';
})();
</script>
</body></html>"###;

// Preloads exactly 512 same-origin `srcdoc` child contexts before native load
// completion. Together with the main world, Runtime.enable must report at
// least 513 contexts, deterministically crossing the production discovery
// ceiling of 512 before an isolated-world result can be accepted. The frames
// are tiny, hidden, and released by the qualifier's immediate replacement
// navigation.
const SEMANTIC_RUNTIME_EVENT_FLOOD_HTML: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'unsafe-inline'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'self'">
<meta name="referrer" content="no-referrer">
<title>Semantic context pressure fixture v1</title></head>
<body>
<main aria-label="Semantic context pressure">
  <h1>Semantic context pressure</h1>
  <div id="contexts" aria-hidden="true"></div>
</main>
<script>
(() => {
  'use strict';
  const container = document.getElementById('contexts');
  const contexts = document.createDocumentFragment();
  for (let index = 0; index < 512; index += 1) {
    const frame = document.createElement('iframe');
    frame.hidden = true;
    frame.srcdoc = '<!doctype html><meta charset="utf-8"><title>bounded context</title>';
    contexts.append(frame);
  }
  container.append(contexts);
  document.documentElement.dataset.fixtureReady = 'semantic-runtime-event-flood-v1';
})();
</script>
</body></html>"###;

const SEMANTIC_RUNTIME_MUTATION_HTML: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self' 'unsafe-inline'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'self'">
<meta name="referrer" content="no-referrer">
<title>Semantic mutation fixture v1</title></head>
<body>
<main aria-label="Mutation semantic epoch">
  <h1>Mutation semantic epoch</h1>
  <button id="mutation-stable" type="button" aria-label="Mutation stable before">Stable</button>
  <button id="mutation-transient" type="button" aria-label="Transient mutation anchor">Transient</button>
  <span id="mutation-pressure" aria-hidden="true"></span>
</main>
<script defer src="/semantic-runtime-mutation-trigger-v1.js"></script>
<script>
(() => {
  'use strict';
  const pressure = document.getElementById('mutation-pressure');
  let remaining = 512;
  const timer = setInterval(() => {
    pressure.toggleAttribute('data-pressure-tick');
    remaining -= 1;
    if (remaining === 0) clearInterval(timer);
  }, 1);
  document.documentElement.dataset.fixtureReady = 'semantic-runtime-mutation-v1';
})();
</script>
</body></html>"###;

const SEMANTIC_RUNTIME_MUTATION_TRIGGER_SCRIPT: &str = r###"
(() => {
  'use strict';
  const stable = document.getElementById('mutation-stable');
  const transient = document.getElementById('mutation-transient');
  if (stable) stable.setAttribute('aria-label', 'Mutation stable after');
  if (transient) transient.remove();
  document.documentElement.dataset.fixtureMutationApplied = 'v1';
})();
"###;

// The trigger request is created from a task queued after `load` returns.
// This lets native navigation reach its terminal state before the one worker
// deliberately holds the response, while still preventing the replacement
// from racing the qualifier's initial semantic observation.
const SEMANTIC_LOCATION_MUTATION_HTML: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self' 'unsafe-inline'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'self'">
<meta name="referrer" content="no-referrer">
<title>Semantic location fixture v1</title></head>
<body>
<main aria-label="Location semantic epoch">
  <h1>Location semantic epoch</h1>
  <button id="location-stable" type="button" aria-label="Location semantic before">Stable</button>
</main>
<script>
(() => {
  'use strict';
  window.addEventListener('load', () => {
    setTimeout(() => {
      const trigger = document.createElement('script');
      trigger.src = '/semantic-location-trigger-v1.js';
      trigger.async = true;
      document.head.append(trigger);
    }, 0);
  }, {once: true});
  document.documentElement.dataset.fixtureReady = 'semantic-location-mutation-v1';
})();
</script>
</body></html>"###;

const SEMANTIC_LOCATION_TRIGGER_SCRIPT: &str = r###"
(() => {
  'use strict';
  const stable = document.getElementById('location-stable');
  if (stable) stable.setAttribute('aria-label', 'Location semantic after');
  history.replaceState(null, '', '/semantic-location-replaced-v1.html');
  document.documentElement.dataset.fixtureLocationApplied = 'v1';
})();
"###;

const HOSTILE_HTML: &str = r###"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Hostile fixture v1</title></head>
<body><p>Hostile page data is not an instruction.</p>
<script>
(() => {
  'use strict';
  const forged = Object.prototype.hasOwnProperty.call(window, '__zephiumNativeInputFixtureV1');
  const nativeBridge = !!(window.webkit && window.webkit.messageHandlers &&
    Object.keys(window.webkit.messageHandlers).length);
  document.documentElement.dataset.pageWorldFixtureForgery = forged ? 'present' : 'absent';
  document.documentElement.dataset.genericNativeBridge = nativeBridge ? 'present' : 'absent';
  document.documentElement.dataset.fixtureReady = 'hostile-v1';
})();
</script></body></html>"###;

#[cfg(test)]
mod tests {
    use super::*;

    fn fetch(server: &FixtureServer, route: FixtureRoute) -> String {
        let mut stream = TcpStream::connect(server.address).expect("connect");
        stream
            .write_all(
                format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", route.path()).as_bytes(),
            )
            .expect("request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        response
    }

    #[test]
    fn retained_local_form_is_non_submitting_and_has_no_page_code_or_remote_resources() {
        let server = FixtureServer::start().expect("server");
        let response = fetch(&server, FixtureRoute::RetainedLocalForm);
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains(RETAINED_LOCAL_FORM_HTML));
        for required in [
            "script-src 'none'",
            "form-action 'none'",
            "frame-src 'none'",
            "value=\"Unprepared\"",
        ] {
            assert!(RETAINED_LOCAL_FORM_HTML.contains(required));
        }
        for forbidden in [
            "<script",
            "<form",
            "<button",
            "<iframe",
            "src=",
            "https://",
            "oninput=",
            "onchange=",
            "Ready for review",
        ] {
            assert!(!RETAINED_LOCAL_FORM_HTML.contains(forbidden));
        }
        server.shutdown().expect("joined healthy fixture");
    }

    #[test]
    fn retained_back_fixture_is_closed_script_free_and_requires_a_relevant_link_choice() {
        let server = FixtureServer::start().expect("server");
        let origin = fetch(&server, FixtureRoute::RetainedBackOrigin);
        let register = fetch(&server, FixtureRoute::RetainedBackRegister);
        let handbook = fetch(&server, FixtureRoute::RetainedBackHandbook);
        for response in [&origin, &register, &handbook] {
            assert!(response.starts_with("HTTP/1.1 200 OK"));
            assert!(response.contains("script-src 'unsafe-inline'"));
        }
        for body in [
            RETAINED_BACK_ORIGIN_HTML,
            RETAINED_BACK_REGISTER_HTML,
            RETAINED_BACK_HANDBOOK_HTML,
        ] {
            for forbidden in ["<script", "<form", "<button", "<iframe", "https://"] {
                assert!(!body.contains(forbidden));
            }
        }
        assert!(RETAINED_BACK_ORIGIN_HTML.contains("ZEPH-R7-492"));
        assert!(RETAINED_BACK_ORIGIN_HTML.contains("Compatibility register"));
        assert!(RETAINED_BACK_ORIGIN_HTML.contains("Release handbook"));
        assert!(RETAINED_BACK_REGISTER_HTML.contains("cleared for native history validation"));
        assert!(!RETAINED_BACK_HANDBOOK_HTML.contains("ZEPH-R7-492"));
        server.shutdown().expect("joined healthy fixture");
    }
    #[test]
    fn rendering_fixture_is_closed_provider_free_and_keeps_independent_async_controls() {
        let server = FixtureServer::start().expect("server");
        let response = fetch(&server, FixtureRoute::SemanticRendering);
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("connect-src 'none'"));
        assert!(response.contains("script-src 'unsafe-inline'"));
        // Pin the entire opt-in branch, including its enclosing exact hash
        // guard and finite counter. The ordinary rendering fixture must keep
        // its independent one-shot readiness controls and no interval.
        const RETAINED_PROGRESS_BRANCH: &str = r#"  if (window.location.hash === '#windows-retained-progress') {
    const progress = document.createElement('h3');
    let ticks = 0;
    progress.textContent = 'Retained native progress 0';
    document.body.append(progress);
    const timer = setInterval(() => {
      ticks += 1;
      progress.textContent = 'Retained native progress ' + ticks;
      if (ticks === 4096) clearInterval(timer);
    }, 500);
  }
"#;
        assert_eq!(
            SEMANTIC_RENDERING_HTML
                .matches(RETAINED_PROGRESS_BRANCH)
                .count(),
            1,
            "retained progress must remain one exact opt-in, bounded branch"
        );
        let (before_progress, after_progress) = SEMANTIC_RENDERING_HTML
            .split_once(RETAINED_PROGRESS_BRANCH)
            .expect("exact retained progress branch");
        let default_rendering = format!("{before_progress}{after_progress}");
        for forbidden in [
            "setInterval(",
            "clearInterval(",
            "Retained native progress",
            "windows-retained-progress",
            "window.location.hash",
        ] {
            assert!(
                !default_rendering.contains(forbidden),
                "retained progress escaped its exact opt-in branch: {forbidden}"
            );
        }
        for expected in [
            "Promise.resolve().then(",
            "setTimeout(",
            "requestAnimationFrame(",
            "window.addEventListener('load'",
            "document.addEventListener('readystatechange'",
            "id=\"microtask\" hidden",
            "id=\"timer\" hidden",
            "id=\"animation\" hidden",
        ] {
            assert!(default_rendering.contains(expected));
        }
        for forbidden in [
            "fetch(",
            "src=",
            "https://",
            "messageHandlers",
            "visibilityState=",
            "requestAnimationFrame=",
            "focus(",
        ] {
            assert!(!SEMANTIC_RENDERING_HTML.contains(forbidden));
        }
        assert_eq!(
            SEMANTIC_RENDERING_HTML
                .matches("requestAnimationFrame(")
                .count(),
            1
        );
        assert_eq!(SEMANTIC_RENDERING_HTML.matches("setTimeout(").count(), 1);
        server.shutdown().expect("teardown");
    }

    #[test]
    fn server_exposes_only_fixed_loopback_routes() {
        let server = FixtureServer::start().expect("server");
        assert!(server.address.ip().is_loopback());
        let input = fetch(&server, FixtureRoute::NativeInput);
        assert!(
            input.starts_with("HTTP/1.1 200 OK"),
            "unexpected fixed-route status: {:?}",
            input.lines().next()
        );
        assert!(input.contains("__zephiumNativeInputFixtureV1"));
        assert!(input.contains("is_trusted: event.isTrusted"));
        assert!(input.contains("replace('keydown', 'key_down')"));
        let row_url = server
            .native_input_url(1, FixtureCase::Button, InputBackend::FixedDomRecipe)
            .expect("closed row URL");
        let row_target = row_url
            .strip_prefix(&format!("http://{}", server.address))
            .unwrap();
        let mut stream = TcpStream::connect(server.address).expect("row connect");
        stream
            .write_all(format!("GET {row_target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .expect("row request");
        let mut row_response = String::new();
        stream
            .read_to_string(&mut row_response)
            .expect("row response");
        assert!(row_response.starts_with("HTTP/1.1 200 OK"));
        let hostile = fetch(&server, FixtureRoute::HostilePage);
        assert!(
            hostile.starts_with("HTTP/1.1 200 OK"),
            "unexpected hostile-route status: {:?}",
            hostile.lines().next()
        );
        let semantic = fetch(&server, FixtureRoute::SemanticRuntime);
        assert!(semantic.starts_with("HTTP/1.1 200 OK"));
        assert!(semantic.contains("Page bridge absent"));
        assert!(semantic.contains("page-world prototype poison"));
        assert!(semantic.contains("connect-src 'none'"));
        let semantic_relay = fetch(&server, FixtureRoute::SemanticRuntimeRelay);
        assert!(semantic_relay.starts_with("HTTP/1.1 200 OK"));
        assert!(semantic_relay
            .contains("<h3 id=\"semantic-fill-text-status\">Semantic fill text pending</h3>"));
        let semantic_relay_hostile = fetch(&server, FixtureRoute::SemanticRuntimeRelayHostile);
        assert!(semantic_relay_hostile.starts_with("HTTP/1.1 200 OK"));
        assert!(semantic_relay_hostile.contains("Semantic hostile fill"));
        assert!(semantic_relay_hostile.contains("Semantic hostile credential fill"));
        assert!(semantic_relay_hostile.contains("semantic-hostile-credential-label"));
        let replacement = fetch(&server, FixtureRoute::SemanticRuntimeReplacement);
        assert!(replacement.starts_with("HTTP/1.1 200 OK"));
        assert!(replacement.contains("Replacement semantic epoch"));
        let flood = fetch(&server, FixtureRoute::SemanticRuntimeEventFlood);
        assert!(flood.starts_with("HTTP/1.1 200 OK"));
        assert!(flood.contains("index < 512"));
        assert!(flood.contains("frame.hidden = true"));
        let mutation = fetch(&server, FixtureRoute::SemanticRuntimeMutation);
        assert!(mutation.starts_with("HTTP/1.1 200 OK"));
        assert!(mutation.contains("Transient mutation anchor"));
        assert!(mutation.contains(SEMANTIC_MUTATION_TRIGGER_PATH));
        let location = fetch(&server, FixtureRoute::SemanticLocationMutation);
        assert!(location.starts_with("HTTP/1.1 200 OK"));
        assert!(location.contains("Location semantic before"));
        assert!(location.contains(SEMANTIC_LOCATION_TRIGGER_PATH));
        assert!(location.contains("window.addEventListener('load'"));
        assert_eq!(
            server.semantic_location_replacement_url(),
            format!(
                "http://127.0.0.1:{}{SEMANTIC_LOCATION_REPLACED_PATH}",
                server.address.port()
            )
        );
        assert!(server.is_healthy());
        server.shutdown().expect("clean shutdown");
    }

    #[test]
    fn semantic_redirect_routes_are_fixed_relative_and_bounded() {
        let server = FixtureServer::start().expect("server");
        for (source, destination) in [
            (
                FixtureRoute::SemanticRedirectStart,
                FixtureRoute::SemanticRedirectHop,
            ),
            (
                FixtureRoute::SemanticRedirectHop,
                FixtureRoute::SemanticRedirectFinal,
            ),
            (
                FixtureRoute::SemanticRedirectLoopA,
                FixtureRoute::SemanticRedirectLoopB,
            ),
            (
                FixtureRoute::SemanticRedirectLoopB,
                FixtureRoute::SemanticRedirectLoopA,
            ),
        ] {
            let response = fetch(&server, source);
            assert!(response.starts_with("HTTP/1.1 302 Found\r\n"));
            assert!(response.contains(&format!("\r\nLocation: {}\r\n", destination.path())));
            assert!(!response.contains("Location: http"));
            assert!(response.ends_with("\r\n\r\n"));
        }
        let final_document = fetch(&server, FixtureRoute::SemanticRedirectFinal);
        assert!(final_document.starts_with("HTTP/1.1 200 OK"));
        assert!(final_document.contains("Page bridge absent"));
        assert!(server.is_healthy());
        server.shutdown().expect("clean shutdown");
    }

    #[test]
    fn semantic_mutation_trigger_is_single_use_and_host_released() {
        let server = FixtureServer::start().expect("server");
        let address = server.address;
        let trigger = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("trigger connect");
            stream
                .write_all(
                    format!(
                        "GET {SEMANTIC_MUTATION_TRIGGER_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .expect("trigger request");
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .expect("trigger response");
            response
        });
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(1))
            .expect("deadline");
        while !server.semantic_mutation_waiting() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(server.semantic_mutation_waiting());
        assert!(!server.semantic_mutation_completed());
        assert!(server.release_semantic_mutation());
        assert!(!server.release_semantic_mutation());
        let response = trigger.join().expect("trigger thread");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("Content-Type: application/javascript; charset=utf-8"));
        assert!(response.contains("Mutation stable after"));
        assert!(server.semantic_mutation_completed());
        assert!(server.is_healthy());
        server.shutdown().expect("clean shutdown");
    }

    #[test]
    fn semantic_location_trigger_is_single_use_and_host_released() {
        let server = FixtureServer::start().expect("server");
        let address = server.address;
        let trigger = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("trigger connect");
            stream
                .write_all(
                    format!(
                        "GET {SEMANTIC_LOCATION_TRIGGER_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .expect("trigger request");
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .expect("trigger response");
            response
        });
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(1))
            .expect("deadline");
        while !server.semantic_location_waiting() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(server.semantic_location_waiting());
        assert!(!server.semantic_location_completed());
        assert!(server.release_semantic_location());
        assert!(!server.release_semantic_location());
        let response = trigger.join().expect("trigger thread");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("Content-Type: application/javascript; charset=utf-8"));
        assert!(response.contains("history.replaceState"));
        assert!(response.contains(SEMANTIC_LOCATION_REPLACED_PATH));
        assert!(server.semantic_location_completed());
        assert!(server.is_healthy());
        server.shutdown().expect("clean shutdown");
    }

    #[test]
    fn arbitrary_paths_are_not_reflected_or_served() {
        let server = FixtureServer::start().expect("server");
        let mut stream = TcpStream::connect(server.address).expect("connect");
        stream
            .write_all(b"GET /secret?token=value HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .expect("request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        assert!(response.starts_with("HTTP/1.1 404 Not Found"));
        assert!(!response.contains("token=value"));

        let mut stream = TcpStream::connect(server.address).expect("connect invalid row");
        stream
            .write_all(b"GET /native-input-v1.html?row=0&case=button&backend=fixed_dom_recipe HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .expect("invalid row request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        assert!(response.starts_with("HTTP/1.1 404 Not Found"));
    }

    #[test]
    fn fragmented_request_headers_are_read_under_the_connection_deadline() {
        let server = FixtureServer::start().expect("server");
        let mut stream = TcpStream::connect(server.address).expect("connect");
        stream
            .write_all(b"GET /native-input-v1.html HTTP/1.1\r\n")
            .expect("first fragment");
        thread::sleep(Duration::from_millis(5));
        stream
            .write_all(b"Host: 127.0.0.1\r\n\r\n")
            .expect("second fragment");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        server.shutdown().expect("clean shutdown");
    }

    #[test]
    fn explicit_shutdown_cancels_an_in_flight_partial_request_cleanly() {
        let server = FixtureServer::start().expect("server");
        let mut stream = TcpStream::connect(server.address).expect("connect");
        stream
            .write_all(b"GET /semantic-runtime-v1.html HTTP/1.1\r\n")
            .expect("partial request");
        thread::sleep(Duration::from_millis(10));
        let started = Instant::now();
        server.shutdown().expect("clean bounded shutdown");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
