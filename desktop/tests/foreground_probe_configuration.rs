#[path = "../foreground_probe_config.rs"]
mod contract;

#[path = "../foreground_probe_admission.rs"]
mod admission;

use admission::{AdmissionDecision, AdmissionGate, ForegroundCheck};
use std::time::{Duration, Instant};

fn sample(gate: &mut AdmissionGate, now: Instant, exact: bool) -> Option<AdmissionDecision> {
    match gate.begin_check(now)? {
        ForegroundCheck::Capture => gate.poll(now, exact),
        ForegroundCheck::DeferredForeground => Some(AdmissionDecision::DeferredForeground),
    }
}

#[test]
fn foreground_wait_allocates_no_admission_before_chrome_and_consumes_once() {
    let now = Instant::now();
    let mut gate = AdmissionGate::default();
    assert!(gate.waiting_chrome());
    assert_eq!(gate.poll(now, true), None);
    assert_eq!(gate.counts(now), (0, 0));
    assert!(gate.begin(now));
    assert_eq!(sample(&mut gate, now, false), Some(AdmissionDecision::Wait));
    assert_eq!(
        sample(&mut gate, now + Duration::from_millis(50), true),
        Some(AdmissionDecision::Admit)
    );
    assert_eq!(
        sample(&mut gate, now + Duration::from_millis(100), true),
        None
    );
    assert!(!gate.begin(now + Duration::from_millis(100)));
}

#[test]
fn foreground_wait_never_extends_its_deadline_or_accepts_late_focus() {
    let now = Instant::now();
    let mut gate = AdmissionGate::default();
    assert!(gate.begin(now));
    assert!(!gate.begin(now + Duration::from_secs(14)));
    assert_eq!(
        sample(&mut gate, now + Duration::from_secs(14), false),
        Some(AdmissionDecision::Wait)
    );
    assert_eq!(
        sample(&mut gate, now + Duration::from_secs(15), true),
        Some(AdmissionDecision::DeferredForeground)
    );
    assert_eq!(sample(&mut gate, now + Duration::from_secs(16), true), None);
}

#[test]
fn foreground_wait_allows_user_selection_after_accessibility_acquisition() {
    let now = Instant::now();
    for elapsed_ms in [5_140, 14_999] {
        let mut gate = AdmissionGate::default();
        assert!(gate.begin(now));
        // Every nominal timer opportunity remains bounded and refuses Work
        // until the user actually selects the exact foreground window.
        for check_ms in (0..elapsed_ms).step_by(50) {
            assert_eq!(
                sample(&mut gate, now + Duration::from_millis(check_ms), false),
                Some(AdmissionDecision::Wait)
            );
        }
        assert_eq!(
            sample(&mut gate, now + Duration::from_millis(elapsed_ms), true),
            Some(AdmissionDecision::Admit)
        );
        assert!(!gate.begin(now + Duration::from_secs(15)));
    }
}

#[test]
fn foreground_wait_cancelled_or_expired_wakes_never_reopen_admission() {
    let now = Instant::now();
    for begun in [false, true] {
        let mut gate = AdmissionGate::default();
        if begun {
            assert!(gate.begin(now));
        }
        gate.close();
        assert_eq!(sample(&mut gate, now, true), None);
        assert!(!gate.begin(now));
        assert!(!gate.awaiting_foreground());
        assert_eq!(gate.counts(now).1, 0);
    }
}

#[test]
fn foreground_wait_has_a_check_ceiling_even_without_clock_progress() {
    let now = Instant::now();
    let mut gate = AdmissionGate::default();
    assert!(gate.begin(now));
    for _ in 0..301 {
        assert_eq!(sample(&mut gate, now, false), Some(AdmissionDecision::Wait));
    }
    assert_eq!(
        sample(&mut gate, now, true),
        Some(AdmissionDecision::DeferredForeground)
    );
    assert_eq!(gate.counts(now).1, 301);
}

#[test]
fn foreground_check_must_be_reserved_and_cannot_cross_deadline_or_cancellation() {
    let now = Instant::now();
    for cancelled in [false, true] {
        let mut gate = AdmissionGate::default();
        assert!(gate.begin(now));
        assert_eq!(gate.poll(now, true), None);
        assert_eq!(
            gate.begin_check(now + Duration::from_secs(14)),
            Some(ForegroundCheck::Capture)
        );
        assert_eq!(gate.begin_check(now + Duration::from_secs(14)), None);
        if cancelled {
            gate.close();
            assert_eq!(gate.poll(now + Duration::from_secs(14), true), None);
        } else {
            assert_eq!(
                gate.poll(now + Duration::from_secs(15), true),
                Some(AdmissionDecision::DeferredForeground)
            );
        }
        assert_eq!(gate.counts(now + Duration::from_secs(15)).1, 1);
    }
}

#[test]
fn rendering_probe_cannot_use_real_application_identity_or_a_development_server() {
    let valid: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.work-rendering-probe.conf.json")).unwrap();
    contract::validate(&valid).unwrap();
    for (pointer, value) in [
        ("/identifier", serde_json::json!("app.quasar")),
        ("/productName", serde_json::json!("Quasar")),
        ("/app/windows/0/title", serde_json::json!("Quasar")),
        ("/build/devUrl", serde_json::json!("http://localhost:1420")),
    ] {
        let mut changed = valid.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(contract::validate(&changed).is_err());
    }
    let mut missing = valid;
    missing.as_object_mut().unwrap().remove("build");
    assert!(contract::validate(&missing).is_err());
}

#[test]
fn rendering_probe_refuses_prior_session_or_profile_data() {
    let empty = tempfile::tempdir().unwrap();
    contract::require_fresh_data_root(empty.path()).unwrap();
    contract::require_fresh_data_root(&empty.path().join("absent")).unwrap();
    std::fs::create_dir(empty.path().join("prior-session")).unwrap();
    assert!(contract::require_fresh_data_root(empty.path()).is_err());
}
