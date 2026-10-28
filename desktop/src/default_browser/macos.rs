//! LaunchServices owns the default browser. Asking changes nothing by itself:
//! macOS shows its own confirmation, and the answer is read back afterwards.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{ns_string, NSBundle, NSError, NSURL};

/// The running bundle, or None outside an installed `.app` (a development
/// binary cannot be registered with LaunchServices).
fn own_bundle() -> Option<Retained<NSURL>> {
    let url = NSBundle::mainBundle().bundleURL();
    url.path()
        .is_some_and(|path| path.to_string().ends_with(".app"))
        .then_some(url)
}

fn same_file(a: &NSURL, b: &NSURL) -> bool {
    let canonical = |url: &NSURL| {
        url.URLByResolvingSymlinksInPath()
            .and_then(|url| url.path())
    };
    match (canonical(a), canonical(b)) {
        (Some(a), Some(b)) => a.to_string() == b.to_string(),
        _ => false,
    }
}

/// The browser macOS opens web links in, unless that is Quasar itself. Sign-in
/// uses it so the person completes it where they are already signed in, and
/// never in a tab of the app that is asking.
pub(crate) fn external_browser() -> Option<Retained<NSURL>> {
    let probe = NSURL::URLWithString(&objc2_foundation::NSString::from_str("https://example.com"))?;
    let handler = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&probe)?;
    let is_quasar = own_bundle().is_some_and(|own| same_file(&handler, &own))
        || handler
            .path()
            .is_some_and(|path| super::names_quasar_app(&path.to_string()));
    (!is_quasar).then_some(handler)
}

pub(crate) fn can_request() -> bool {
    own_bundle().is_some()
}

pub(crate) fn is_default() -> bool {
    let Some(own) = own_bundle() else {
        return false;
    };
    let workspace = NSWorkspace::sharedWorkspace();
    ["http://example.com", "https://example.com"]
        .iter()
        .all(|probe| {
            NSURL::URLWithString(&objc2_foundation::NSString::from_str(probe))
                .and_then(|probe| workspace.URLForApplicationToOpenURL(&probe))
                .is_some_and(|handler| same_file(&handler, &own))
        })
}

/// Asks for both web schemes. `done` runs once macOS has its answer, on a
/// thread of its choosing; it runs exactly once even when nothing was asked.
pub(crate) fn request(done: Box<dyn FnOnce() + Send>) {
    let Some(own) = own_bundle() else {
        done();
        return;
    };
    let workspace = NSWorkspace::sharedWorkspace();
    let done = std::sync::Mutex::new(Some(done));
    let https_own = own.clone();
    let https = RcBlock::new(move |_error: *mut NSError| {
        if let Some(done) = done.lock().ok().and_then(|mut slot| slot.take()) {
            done();
        }
    });
    // http carries the consent prompt and makes Zephium the default web
    // browser; https is set after it so both schemes agree.
    let http = RcBlock::new(move |_error: *mut NSError| {
        NSWorkspace::sharedWorkspace()
            .setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
                &https_own,
                ns_string!("https"),
                Some(&https),
            );
    });
    workspace.setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
        &own,
        ns_string!("http"),
        Some(&http),
    );
}
