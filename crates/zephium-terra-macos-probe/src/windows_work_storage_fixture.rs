//! The parent owns a fresh QA journal; the replica child owns its process fence.
use super::ProbeFailure;
use std::{
    ffi::OsStr,
    io::{Read as _, Write as _},
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, ExitStatus, Stdio},
    time::{Duration, Instant},
};
use zephium_private_fs::{NativeSession, NativeWorkStorageTestSession};

const SESSION_ENV: &str = "ZEPHIUM_WINDOWS_REPLICA_JOURNAL_SESSION";
const CHILD_BUDGET: Duration = Duration::from_secs(930);

struct OwnedChild {
    child: Child,
    reaped: bool,
}

impl OwnedChild {
    fn wait_bounded(&mut self) -> Result<ExitStatus, ProbeFailure> {
        let deadline = Instant::now() + CHILD_BUDGET;
        loop {
            if let Some(status) = self.child.try_wait().map_err(|_| ProbeFailure::Runtime)? {
                self.reaped = true;
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(ProbeFailure::Runtime);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            // Closing the sole parent writer also signals the child's watcher.
            self.child.stdin.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn watch_parent() -> Result<(), ProbeFailure> {
    let mut input = std::io::stdin();
    let mut byte = [0];
    input
        .read_exact(&mut byte)
        .map_err(|_| ProbeFailure::Authority)?;
    if byte != [1] {
        return Err(ProbeFailure::Authority);
    }
    std::thread::Builder::new()
        .name("replica-parent-liveness".into())
        .spawn(move || {
            // EOF also follows forced parent termination; exit independently
            // of a potentially blocked native/model workflow on another thread.
            let _ = input.read(&mut byte);
            std::process::exit(1);
        })
        .map_err(|_| ProbeFailure::Runtime)?;
    Ok(())
}

fn run_child(scenario: &OsStr, label: &str) -> Result<ExitStatus, ProbeFailure> {
    let executable = std::env::current_exe().map_err(|_| ProbeFailure::Runtime)?;
    let mut owned = OwnedChild {
        child: std::process::Command::new(executable)
            .arg("--loopback-site")
            .arg(scenario)
            .env(SESSION_ENV, label)
            .stdin(Stdio::piped())
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|_| ProbeFailure::Runtime)?,
        reaped: false,
    };
    owned
        .child
        .stdin
        .as_mut()
        .ok_or(ProbeFailure::Runtime)?
        .write_all(&[1])
        .map_err(|_| ProbeFailure::Runtime)?;
    // Child::wait closes stdin first, so use try_wait while the watcher owns
    // the read end. The writer lives until confirmed exit or owned cancellation.
    owned.wait_bounded()
}

pub(super) fn run(scenario: &OsStr) -> Result<(), ProbeFailure> {
    if let Some(label) = std::env::var_os(SESSION_ENV) {
        // Only the creating parent retires the scope. The child keeps Work's
        // original journal lease until actual OS exit, including Store shutdown.
        let label = label.to_str().ok_or(ProbeFailure::Authority)?;
        NativeSession::new(label).map_err(|_| ProbeFailure::Authority)?;
        watch_parent()?;
        return super::work_site::run(scenario);
    }
    let label = format!("work-probe-{}", zephium_core::ids::ProfileId::generate());
    let session = NativeSession::new(&label).map_err(|_| ProbeFailure::Authority)?;
    let owner =
        NativeWorkStorageTestSession::create(&session).map_err(|_| ProbeFailure::Runtime)?;
    let result = run_child(scenario, &label);
    // This is deliberately after wait/status, not after the child's Store
    // closes. The OS must release the permanent process fence first.
    owner.retire().map_err(|_| ProbeFailure::Runtime)?;
    let successful = result?.success();
    let _ = writeln!(
        std::io::stdout().lock(),
        "windows-work-journal-fixture: exact_session_retired=true child_success={successful}; content=synthetic"
    );
    if successful {
        Ok(())
    } else {
        Err(ProbeFailure::Runtime)
    }
}

pub(super) fn open_store(directory: &Path) -> Result<zephium_store::SqliteStore, ProbeFailure> {
    let label = std::env::var(SESSION_ENV).map_err(|_| ProbeFailure::Authority)?;
    let selector =
        zephium_store::WindowsWorkStorage::for_application("app.quasar.webext-qa", Some(&label))
            .map_err(|_| ProbeFailure::Authority)?;
    zephium_store::SqliteStore::open_with_windows_work_storage(directory, selector)
        .map_err(|_| ProbeFailure::Runtime)
}
