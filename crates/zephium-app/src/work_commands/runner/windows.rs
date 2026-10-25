//! LPAC backend. Token/ACL and actual Git-distribution qualification is pending.
//! The production gate remains closed independently of source compilation.
use super::{Failure, GitOperation, Output, Snapshot};
use ::windows::{
    core::{BOOL, PCWSTR, PWSTR},
    Win32::{
        Foundation::{LocalFree, HANDLE, HLOCAL},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetSecurityInfo, SetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            CopySid, FreeSid, GetLengthSid, GetSecurityDescriptorDacl, GetSecurityDescriptorSacl,
            GetTokenInformation,
            Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile},
            TokenUser, DACL_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION,
            OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            PSID, SECURITY_CAPABILITIES, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
};
use std::{
    ffi::{OsStr, OsString},
    fs,
    future::Future,
    mem::{size_of, size_of_val},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    time::Duration,
};

fn wide(value: &OsStr) -> Result<Vec<u16>, Failure> {
    let mut wide: Vec<_> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(Failure::Input);
    }
    wide.push(0);
    Ok(wide)
}
fn sid_string(sid: PSID) -> Result<String, Failure> {
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(|_| Failure::Storage)?;
    let result = unsafe { text.to_string() }.map_err(|_| Failure::Storage);
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    result
}
fn owner_sid() -> Result<String, Failure> {
    let mut raw = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) }
        .map_err(|_| Failure::Storage)?;
    let token = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let mut buffer = [0usize; 128];
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            size_of_val(&buffer) as u32,
            &mut length,
        )
    }
    .map_err(|_| Failure::Storage)?;
    if length < size_of::<TOKEN_USER>() as u32 || length as usize > size_of_val(&buffer) {
        return Err(Failure::Storage);
    }
    sid_string(unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid })
}

/// Seal an empty fresh root before the broker copies any granted content into it.
pub(super) fn protect_root(path: &Path) -> Result<(), Failure> {
    let owner = owner_sid()?;
    let sddl = wide(OsStr::new(&format!(
        "D:P(A;OICI;FA;;;{owner})(A;OICI;FA;;;SY)"
    )))?;
    // Seal the exact opened directory, never a path which a junction swap could
    // redirect between creation and ACL assignment. No payload is copied yet.
    let file = fs::OpenOptions::new()
        .access_mode(0x0006_0080)
        .share_mode(0)
        .custom_flags(0x0220_0000)
        .open(path)
        .map_err(|_| Failure::Storage)?;
    let metadata = file.metadata().map_err(|_| Failure::Storage)?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(Failure::Storage);
    }
    let mut actual_owner = PSID::default();
    let mut current_descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut actual_owner),
            None,
            None,
            None,
            Some(&mut current_descriptor),
        )
    }
    .ok()
    .map_err(|_| Failure::Storage)?;
    let actual = sid_string(actual_owner);
    unsafe { LocalFree(Some(HLOCAL(current_descriptor.0))) };
    if actual? != owner {
        return Err(Failure::Storage);
    }
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|_| Failure::Storage)?;
    let assigned = assign_security(&file, descriptor, false);
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    assigned
}

pub(super) struct Profile {
    name: Vec<u16>,
    sid: Vec<usize>,
}
impl Profile {
    pub(super) fn create() -> Result<Self, Failure> {
        let name = wide(OsStr::new(&format!(
            "Quasar.Work.{}",
            zephium_core::work::WorkArtifactId::generate()
        )))?;
        let sid = unsafe {
            CreateAppContainerProfile(
                PCWSTR(name.as_ptr()),
                PCWSTR(name.as_ptr()),
                PCWSTR(name.as_ptr()),
                None,
            )
        }
        .map_err(|_| Failure::Unavailable)?;
        let length = unsafe { GetLengthSid(sid) } as usize;
        if !(8..=128).contains(&length) {
            unsafe {
                FreeSid(sid);
                let _ = DeleteAppContainerProfile(PCWSTR(name.as_ptr()));
            }
            return Err(Failure::Storage);
        }
        let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
        let copied = unsafe { CopySid(length as u32, PSID(storage.as_mut_ptr().cast()), sid) };
        unsafe {
            FreeSid(sid);
        }
        if copied.is_err() {
            unsafe {
                let _ = DeleteAppContainerProfile(PCWSTR(name.as_ptr()));
            }
            return Err(Failure::Storage);
        }
        Ok(Self { name, sid: storage })
    }
    fn capabilities(&self) -> SECURITY_CAPABILITIES {
        SECURITY_CAPABILITIES {
            AppContainerSid: PSID(self.sid.as_ptr().cast_mut().cast()),
            Capabilities: std::ptr::null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        }
    }
    pub(super) fn grant_copy(&self, snapshot: &Snapshot, read_only: bool) -> Result<(), Failure> {
        let owner = owner_sid()?;
        let package = sid_string(PSID(self.sid.as_ptr().cast_mut().cast()))?;
        let mut pending = vec![snapshot.root.clone()];
        let mut count = 0;
        while let Some(path) = pending.pop() {
            count += 1;
            if count > 8_192 {
                return Err(Failure::Limit);
            }
            let metadata = fs::symlink_metadata(&path).map_err(|_| Failure::Storage)?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(Failure::Storage);
            }
            // Hold an exclusive no-follow handle through the ACL update. A
            // staged hardlink must never delegate access to an outside inode.
            let file = fs::OpenOptions::new()
                .access_mode(0x000c_0080) // WRITE_DAC | WRITE_OWNER | FILE_READ_ATTRIBUTES
                .share_mode(0)
                .custom_flags(0x0220_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
                .open(&path)
                .map_err(|_| Failure::Storage)?;
            let metadata = file.metadata().map_err(|_| Failure::Storage)?;
            if metadata.file_attributes() & 0x400 != 0
                || (!metadata.is_dir()
                    && (!metadata.is_file()
                        || !crate::work_files::single_link(&file).map_err(|_| Failure::Storage)?))
            {
                return Err(Failure::Storage);
            }
            let writable = !read_only
                && (path.starts_with(snapshot.work.join(".git"))
                    || path.starts_with(&snapshot.temporary));
            let access = if writable { "FA" } else { "GRGX" };
            let sddl = wide(OsStr::new(&format!(
                "D:P(A;OICI;FA;;;{owner})(A;OICI;FA;;;SY)(A;OICI;{access};;;{package})S:(ML;OICI;NW;;;LW)"
            )))?;
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl.as_ptr()),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )
            }
            .map_err(|_| Failure::Storage)?;
            let assigned = assign_security(&file, descriptor, true);
            unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
            assigned?;
            drop(file);
            if metadata.is_dir() {
                for entry in fs::read_dir(path).map_err(|_| Failure::Storage)? {
                    pending.push(entry.map_err(|_| Failure::Storage)?.path());
                }
            }
        }
        Ok(())
    }
}
fn assign_security(
    file: &fs::File,
    descriptor: PSECURITY_DESCRIPTOR,
    label: bool,
) -> Result<(), Failure> {
    let mut present = BOOL::default();
    let mut defaulted = BOOL::default();
    let mut dacl = std::ptr::null_mut();
    let mut sacl = std::ptr::null_mut();
    unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        .map_err(|_| Failure::Storage)?;
    if !present.as_bool() || dacl.is_null() {
        return Err(Failure::Storage);
    }
    if label {
        unsafe { GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted) }
            .map_err(|_| Failure::Storage)?;
        if !present.as_bool() || sacl.is_null() {
            return Err(Failure::Storage);
        }
    }
    let information = DACL_SECURITY_INFORMATION
        | PROTECTED_DACL_SECURITY_INFORMATION
        | if label {
            LABEL_SECURITY_INFORMATION
        } else {
            Default::default()
        };
    unsafe {
        SetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            information,
            None,
            None,
            Some(dacl),
            if label { Some(sacl) } else { None },
        )
    }
    .ok()
    .map_err(|_| Failure::Storage)
}

impl Drop for Profile {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteAppContainerProfile(PCWSTR(self.name.as_ptr()));
        }
    }
}

pub(super) async fn git<C, CF>(
    snapshot: &Snapshot,
    operation: GitOperation,
    cancelled: &C,
) -> Result<Output, Failure>
where
    C: Fn() -> CF,
    CF: Future<Output = bool>,
{
    // This path is broker-owned. Native qualification must establish a packaged
    // Git and DLL closure here; ambient PATH/Program Files credentials are never
    // granted to the process. Until then there is no usable Windows Git backend.
    let program = snapshot.root.join("tool/git.exe");
    if !program.is_file() {
        return Err(Failure::ToolUnavailable);
    }
    run(
        snapshot,
        &program,
        &operation.arguments(snapshot),
        cancelled,
    )
    .await
}

async fn run<C, CF>(
    snapshot: &Snapshot,
    program: &Path,
    arguments: &[OsString],
    cancelled: &C,
) -> Result<Output, Failure>
where
    C: Fn() -> CF,
    CF: Future<Output = bool>,
{
    run_mode(snapshot, program, arguments, cancelled, false).await
}

pub(super) async fn python<C, CF>(
    snapshot: &Snapshot,
    modules: &[String],
    cancelled: &C,
) -> Result<Output, Failure>
where
    C: Fn() -> CF,
    CF: Future<Output = bool>,
{
    // A future qualified package broker must supply this complete pinned runtime
    // closure. Ambient PATH, installed user Python and user site-packages are
    // never admitted. The shared Windows availability gate remains closed.
    let runtime = snapshot.root.join("tool/python");
    let program = runtime.join("python.exe");
    if !program.is_file() {
        return Err(Failure::PythonUnavailable);
    }
    let request = serde_json::json!({"platform":"windows","root":snapshot.work,"home":snapshot.home,"temporary":snapshot.temporary,"stdlib":[runtime.join("Lib"),runtime.join("DLLs")],"modules":modules});
    let arguments = vec![
        "-I".into(),
        "-S".into(),
        "-B".into(),
        "-c".into(),
        super::python::BOOTSTRAP.into(),
        request.to_string().into(),
    ];
    run_mode(snapshot, &program, &arguments, cancelled, true).await
}

async fn run_mode<C, CF>(
    snapshot: &Snapshot,
    program: &Path,
    arguments: &[OsString],
    cancelled: &C,
    read_only: bool,
) -> Result<Output, Failure>
where
    C: Fn() -> CF,
    CF: Future<Output = bool>,
{
    if super::cancellation_requested(cancelled).await {
        return Err(Failure::Stopped);
    }
    if !program.starts_with(snapshot.root.join("tool")) {
        return Err(Failure::Input);
    }
    let profile = Profile::create()?;
    profile.grant_copy(snapshot, read_only)?;
    #[allow(unused_mut)]
    let mut environment = vec![
        ("HOME".into(), snapshot.home.as_os_str().into()),
        ("USERPROFILE".into(), snapshot.home.as_os_str().into()),
        ("TMP".into(), snapshot.temporary.as_os_str().into()),
        ("TEMP".into(), snapshot.temporary.as_os_str().into()),
        ("XDG_CONFIG_HOME".into(), snapshot.home.as_os_str().into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_CONFIG_GLOBAL".into(), "NUL".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_ATTR_NOSYSTEM".into(), "1".into()),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
        ("LC_ALL".into(), "C".into()),
        ("CI".into(), "1".into()),
    ];
    #[cfg(test)]
    if program
        .file_name()
        .is_some_and(|name| name == "native-fixture.exe")
    {
        environment.push((
            "ZEPHIUM_RUNNER_FIXTURE".into(),
            snapshot.temporary.join("fixture.json").into_os_string(),
        ));
    }
    run_prepared(
        snapshot,
        program,
        arguments,
        environment,
        &profile,
        cancelled,
    )
    .await
}

pub(super) async fn run_prepared<C, CF>(
    snapshot: &Snapshot,
    program: &Path,
    arguments: &[OsString],
    environment: Vec<(OsString, OsString)>,
    profile: &Profile,
    cancelled: &C,
) -> Result<Output, Failure>
where
    C: Fn() -> CF,
    CF: Future<Output = bool>,
{
    if !program.starts_with(snapshot.root.join("tool")) {
        return Err(Failure::Input);
    }
    if super::cancellation_requested(cancelled).await {
        return Err(Failure::Stopped);
    }
    let mut process = {
        let capabilities = profile.capabilities();
        crate::work_commands::windows::spawn_program(
            &snapshot.work,
            program,
            arguments,
            environment,
            Some(&capabilities),
        )
        .map_err(|_| Failure::Launch)?
    };
    let deadline = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut output = Vec::new();
    let mut closed = false;
    let result = loop {
        tokio::select! {
            bytes = process.output.recv(), if !closed => match bytes {
                Some(Ok(bytes)) => { output.extend(bytes); if output.len() > 64 * 1024 { break Err(Failure::Limit); } },
                Some(Err(())) => break Err(Failure::Failed), None => closed = true,
            },
            _ = &mut deadline => break Err(Failure::Stopped),
            _ = tick.tick() => {
                if process.quitting() || super::cancellation_requested(cancelled).await { break Err(Failure::Stopped); }
                match process.status().map_err(|_| Failure::Failed) {
                    Ok(Some(code)) if closed => break Ok(code),
                    Err(error) => break Err(error), _ => {}
                }
            }
        }
    };
    process.stop();
    let settled = tokio::time::timeout(Duration::from_millis(300), async {
        while process.status().ok().flatten().is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    drop(process);
    if settled.is_err() {
        return Err(Failure::StopUnconfirmed);
    }
    let exit = result?;
    let text = super::output_text(output)?;
    Ok(Output {
        text,
        success: exit == 0,
        exit: exit as i32,
    })
}

impl Profile {
    pub(super) fn grant_socket(
        &self,
        snapshot: &Snapshot,
        path: &Path,
    ) -> Result<fs::File, Failure> {
        if path != snapshot.root.join("broker.sock") {
            return Err(Failure::Input);
        }
        let file = fs::OpenOptions::new()
            .access_mode(0x000c_0080)
            .share_mode(3)
            .custom_flags(0x0020_0000)
            .open(path)
            .map_err(|_| Failure::Storage)?;
        Self::verify_socket(&file)?;
        let owner = owner_sid()?;
        let package = sid_string(PSID(self.sid.as_ptr().cast_mut().cast()))?;
        let sddl = wide(OsStr::new(&format!(
            "D:P(A;;FA;;;{owner})(A;;FA;;;SY)(A;;GRGW;;;{package})S:(ML;;NW;;;LW)"
        )))?;
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|_| Failure::Storage)?;
        let assigned = assign_security(&file, descriptor, true);
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        assigned?;
        Self::verify_socket(&file)?;
        Ok(file)
    }
    pub(super) fn verify_socket(file: &fs::File) -> Result<(), Failure> {
        use ::windows::Win32::Storage::FileSystem::{
            FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO,
        };
        let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileAttributeTagInfo,
                (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        }
        .map_err(|_| Failure::Storage)?;
        // IO_REPARSE_TAG_AF_UNIX from the Windows SDK; no symlink/junction exception.
        if tag.ReparseTag != 0x8000_0023
            || tag.FileAttributes & 0x400 == 0
            || tag.FileAttributes & 0x10 != 0
            || !crate::work_files::single_link(file).map_err(|_| Failure::Storage)?
        {
            return Err(Failure::Storage);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::{TcpListener, TcpStream},
        time::Instant,
    };

    #[test]
    #[ignore = "Fixture child runs only inside the parent LPAC qualification test"]
    fn native_lpac_fixture_child() {
        let Some(manifest) = std::env::var_os("ZEPHIUM_RUNNER_FIXTURE") else {
            return;
        };
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
        if value["hold"].as_bool() == Some(true) {
            fs::write(".git/ready", "ready").unwrap();
            std::thread::sleep(Duration::from_secs(60));
            panic!("fixture was not stopped");
        }
        assert_eq!(fs::read("notes.txt").unwrap(), b"inert inside fixture\n");
        assert!(fs::write("notes.txt", "forbidden").is_err());
        assert!(fs::read(value["outside"].as_str().unwrap()).is_err());
        assert!(fs::write(value["outside"].as_str().unwrap(), "forbidden").is_err());
        assert!(fs::read_dir(value["outside_root"].as_str().unwrap()).is_err());
        assert!(TcpStream::connect(value["address"].as_str().unwrap()).is_err());
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .spawn()
            .is_err());
        fs::write(
            std::env::temp_dir().join("allowed.txt"),
            "inert scratch fixture",
        )
        .unwrap();
        assert!(std::env::var_os("GITHUB_TOKEN").is_none());
        assert!(std::env::var_os("SSH_AUTH_SOCK").is_none());
    }

    fn fixture(
        snapshot: &Snapshot,
        outside: &Path,
        address: &str,
        hold: bool,
    ) -> std::path::PathBuf {
        fs::create_dir(snapshot.root.join("tool")).unwrap();
        let program = snapshot.root.join("tool/native-fixture.exe");
        fs::copy(std::env::current_exe().unwrap(), &program).unwrap();
        fs::write(snapshot.temporary.join("fixture.json"), serde_json::to_vec(&serde_json::json!({
            "outside": outside.join("sentinel.txt"), "outside_root": outside, "address": address, "hold": hold,
        })).unwrap()).unwrap();
        program
    }
    fn args() -> Vec<OsString> {
        [
            "--exact",
            "work_commands::runner::windows::tests::native_lpac_fixture_child",
            "--ignored",
            "--nocapture",
        ]
        .into_iter()
        .map(Into::into)
        .collect()
    }

    #[tokio::test]
    #[ignore = "Requires physical Windows: LPAC token, ACL, no-network and process limits"]
    async fn native_lpac_denies_outside_access_and_children() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("sentinel.txt"), "inert outside fixture").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let snapshot = Snapshot::fixture(&[("notes.txt", b"inert inside fixture\n")]);
        let program = fixture(
            &snapshot,
            outside.path(),
            &listener.local_addr().unwrap().to_string(),
            false,
        );
        let output = run(&snapshot, &program, &args(), &|| async { false })
            .await
            .unwrap();
        assert!(output.success);
        assert_eq!(
            fs::read_to_string(outside.path().join("sentinel.txt")).unwrap(),
            "inert outside fixture"
        );
        assert_eq!(
            fs::read_to_string(snapshot.work.join("notes.txt")).unwrap(),
            "inert inside fixture\n"
        );
    }

    #[tokio::test]
    #[ignore = "Requires physical Windows: confined process cancellation and job settlement"]
    async fn native_lpac_stop_settles_under_one_second() {
        let outside = tempfile::tempdir().unwrap();
        let snapshot = Snapshot::fixture(&[]);
        let program = fixture(&snapshot, outside.path(), "127.0.0.1:1", true);
        let cancelled_at = std::sync::Mutex::new(None::<Instant>);
        let result = run(&snapshot, &program, &args(), &|| async {
            if snapshot.work.join(".git/ready").exists() {
                cancelled_at
                    .lock()
                    .unwrap()
                    .get_or_insert_with(Instant::now);
                true
            } else {
                false
            }
        })
        .await;
        assert!(matches!(result, Err(Failure::Stopped)));
        assert!(
            cancelled_at
                .lock()
                .unwrap()
                .expect("fixture reached running state")
                .elapsed()
                < Duration::from_secs(1)
        );
    }

    #[test]
    fn process_future_does_not_retain_thread_bound_sid_pointers() {
        fn send<T: Send>(_: T) {}
        let snapshot = Snapshot::fixture(&[]);
        let cancelled = || async { false };
        send(run(
            &snapshot,
            &snapshot.root.join("tool/git.exe"),
            &[],
            &cancelled,
        ));
    }
}
