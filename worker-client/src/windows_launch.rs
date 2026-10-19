//! AppContainer + Job Object Windows runner. The containment is created
//! before the initial thread resumes. No ordinary-user-process fallback.
//!
//! This is a research gate: the production release additionally needs a
//! trusted installation directory with an immutable ACL and code signing.
use super::{ClientError, WorkerSettings};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr::{null, null_mut};
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{FreeSid, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
    TerminateJobObject, JobObjectExtendedLimitInformation,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_LIMIT_PROCESS_TIME,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, STARTUPINFOW,
};

const PROFILE_NAME: &str = "AkulaShivaWorkerNoNetworkV1";
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const JOB_MEMORY_BYTES: usize = 512 * 1024 * 1024;

fn fail() -> io::Error { io::Error::last_os_error() }
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Owning handle; handle inheritance is never assumed from untrusted sources.
struct Owned(HANDLE);
impl Owned {
    fn new(h: HANDLE) -> io::Result<Self> {
        if h.is_null() || h == INVALID_HANDLE_VALUE { Err(fail()) } else { Ok(Self(h)) }
    }
    fn raw(&self) -> HANDLE { self.0 }
    fn file(mut self) -> File {
        let handle = self.0;
        self.0 = null_mut();
        unsafe { File::from_raw_handle(handle as _) }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0); }
        }
    }
}
struct Sid(PSID);
impl Drop for Sid {
    fn drop(&mut self) { if !self.0.is_null() { unsafe { FreeSid(self.0); } } }
}
struct Attrs(windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST);
impl Drop for Attrs {
    fn drop(&mut self) { unsafe { DeleteProcThreadAttributeList(self.0); } }
}

/// Keep this file open with sharing limited to read-only until after
/// CreateProcessW. This blocks normal file writes/rename during launch.
/// Parent-directory reparse-point changes are still a packaging trust gate.
pub(crate) fn pin_executable(settings: &WorkerSettings) -> Result<File, ClientError> {
    let path = &settings.executable;
    if !path.is_absolute() { return Err(ClientError::InvalidConfiguration); }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ClientError::Io)?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > MAX_BINARY_BYTES {
        return Err(ClientError::InvalidConfiguration);
    }
    let mut file = OpenOptions::new()
        .read(true).share_mode(FILE_SHARE_READ)
        .open(path).map_err(|_| ClientError::Io)?;
    let mut hash = blake3::Hasher::new();
    let mut buf = [0u8; 16 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|_| ClientError::Io)?;
        if n == 0 { break; }
        total = total.checked_add(n as u64).ok_or(ClientError::InvalidConfiguration)?;
        if total > MAX_BINARY_BYTES { return Err(ClientError::InvalidConfiguration); }
        hash.update(&buf[..n]);
    }
    if total != metadata.len() { return Err(ClientError::InvalidConfiguration); }
    if hash.finalize().to_hex().as_str() != settings.expected_binary_blake3 {
        return Err(ClientError::WorkerHashMismatch);
    }
    Ok(file)
}
static APP_CONTAINER_PROFILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn profile_sid() -> io::Result<Sid> {
    // Multiple parallel extraction calls must not race first-time Windows
    // profile provisioning within the same user account/process.
    let _serialization = APP_CONTAINER_PROFILE_LOCK.lock()
        .map_err(|_| io::Error::other("AppContainer profile lock poisoned"))?;
    let name = wide(OsStr::new(PROFILE_NAME));
    let display = wide(OsStr::new("Akula Shiva sandbox (no capabilities)"));
    let mut sid = null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(name.as_ptr(), display.as_ptr(), display.as_ptr(),
            null(), 0, &mut sid)
    };
    if hr < 0 {
        // HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)
        if hr as u32 != 0x8007_00B7 {
            return Err(io::Error::other(format!(
                "AppContainer profile unavailable (HRESULT=0x{:08X})", hr as u32
            )));
        }
        let derived = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        if derived < 0 {
            return Err(io::Error::other(format!(
                "AppContainer SID derivation failed (HRESULT=0x{:08X})", derived as u32
            )));
        }
    }
    if sid.is_null() { return Err(io::Error::other("empty AppContainer SID")); }
    Ok(Sid(sid))
}
fn create_job() -> io::Result<Owned> {
    let job = Owned::new(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_JOB_MEMORY
        | JOB_OBJECT_LIMIT_PROCESS_TIME;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.BasicLimitInformation.PerProcessUserTimeLimit = 15 * 10_000_000;
    limits.ProcessMemoryLimit = JOB_MEMORY_BYTES;
    limits.JobMemoryLimit = JOB_MEMORY_BYTES;
    let ok = unsafe { SetInformationJobObject(job.raw(), JobObjectExtendedLimitInformation,
        (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
        size_of_val(&limits) as u32) };
    if ok == 0 { return Err(fail()); }
    Ok(job)
}
fn make_pipe() -> io::Result<(Owned, Owned)> {
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(), bInheritHandle: 1,
    };
    let mut reader = null_mut();
    let mut writer = null_mut();
    if unsafe { CreatePipe(&mut reader, &mut writer, &mut sa, 0) } == 0 { return Err(fail()); }
    Ok((Owned::new(reader)?, Owned::new(writer)?))
}

/// Send only a narrow, known command line to the trusted binary.
/// The path itself is passed separately as lpApplicationName.
fn command_line(executable: &Path, args: &[String]) -> io::Result<Vec<u16>> {
    let mut out = Vec::new();
    fn push_arg(out: &mut Vec<u16>, arg: &OsStr) -> io::Result<()> {
        let units: Vec<u16> = arg.encode_wide().collect();
        if units.iter().any(|x| *x == 0 || *x == b'"' as u16)
            || units.last() == Some(&(b'\\' as u16)) {
            return Err(io::Error::other("command line contains forbidden characters"));
        }
        out.push(b'"' as u16);
        out.extend(units);
        out.push(b'"' as u16);
        Ok(())
    }
    push_arg(&mut out, executable.as_os_str())?;
    for arg in args {
        out.push(b' ' as u16);
        push_arg(&mut out, OsStr::new(arg))?;
    }
    out.push(0);
    Ok(out)
}
pub(crate) struct WindowsChild {
    _job: Owned,
    process: Owned,
    pub(crate) stdin: Option<File>,
    pub(crate) stdout: Option<File>,
}
impl WindowsChild {
    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let wait = unsafe { WaitForSingleObject(self.process.raw(), 0) };
        if wait == WAIT_TIMEOUT { return Ok(None); }
        if wait != WAIT_OBJECT_0 { return Err(fail()); }
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.process.raw(), &mut code) } == 0 {
            return Err(fail());
        }
        Ok(Some(ExitStatus::from_raw(code)))
    }
    pub(crate) fn kill(&mut self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self._job.raw(), 1) } == 0 { return Err(fail()); }
        Ok(())
    }
    pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
        let wait = unsafe { WaitForSingleObject(self.process.raw(), 30_000) };
        if wait != WAIT_OBJECT_0 { return Err(fail()); }
        self.try_wait()?.ok_or_else(|| io::Error::other("worker did not terminate"))
    }
}
/// A live Job Object handle kills the process tree if the host drops the child.
pub(crate) fn spawn(executable: &Path, args: &[String]) -> io::Result<WindowsChild> {
    if !executable.is_absolute() { return Err(io::Error::other("worker path must be absolute")); }
    let sid = profile_sid().map_err(|e| io::Error::other(format!("profile setup: {e}")))?;
    let job = create_job().map_err(|e| io::Error::other(format!("job setup: {e}")))?;
    let (stdin_read, stdin_write) = make_pipe()
        .map_err(|e| io::Error::other(format!("stdin pipe: {e}")))?;
    let (stdout_read, stdout_write) = make_pipe()
        .map_err(|e| io::Error::other(format!("stdout pipe: {e}")))?;
    // Win32 NUL is a device handle rather than a path in the user's vault.
    let nul = wide(OsStr::new("NUL"));
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(), bInheritHandle: 1,
    };
    let stderr = Owned::new(unsafe { CreateFileW(nul.as_ptr(), 0x4000_0000,
        FILE_SHARE_READ | FILE_SHARE_WRITE, &mut sa, OPEN_EXISTING, 0, null_mut()) })?;
    for h in [&stdin_write, &stdout_read] {
        if unsafe { SetHandleInformation(h.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(fail());
        }
    }
    let mut allow = [stdin_read.raw(), stdout_write.raw(), stderr.raw()];
    // All pointed-to attribute inputs must outlive CreateProcessW. Never
    // return pointers to stack-local SECURITY_CAPABILITIES from a helper.
    let mut attr_bytes = 0usize;
    unsafe { InitializeProcThreadAttributeList(null_mut(), 3, 0, &mut attr_bytes); }
    if attr_bytes == 0 || attr_bytes > 128 * 1024 {
        return Err(io::Error::other("process attribute list unavailable"));
    }
    let mut attr_storage = vec![0usize; attr_bytes.div_ceil(size_of::<usize>())];
    let list = attr_storage.as_mut_ptr() as *mut core::ffi::c_void;
    if unsafe { InitializeProcThreadAttributeList(list, 3, 0, &mut attr_bytes) } == 0 {
        return Err(fail());
    }
    let attr_guard = Attrs(list);
    let security_caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0, Capabilities: null_mut(), CapabilityCount: 0, Reserved: 0,
    };
    let child_policy = 0x01u32; // PROCESS_CREATION_CHILD_PROCESS_RESTRICTED
    if unsafe { UpdateProcThreadAttribute(list, 0,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
        (&security_caps as *const SECURITY_CAPABILITIES).cast(),
        size_of_val(&security_caps), null_mut(), null()) } == 0 {
        return Err(fail());
    }
    if unsafe { UpdateProcThreadAttribute(list, 0,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        allow.as_mut_ptr().cast(), size_of_val(&allow), null_mut(), null()) } == 0 {
        return Err(fail());
    }
    if unsafe { UpdateProcThreadAttribute(list, 0,
        PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY as usize,
        (&child_policy as *const u32).cast(),
        size_of_val(&child_policy), null_mut(), null()) } == 0 {
        return Err(fail());
    }
    let mut cmd = command_line(executable, args)?;
    let program = wide(executable.as_os_str());
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other("SystemRoot unavailable"))?;
    let directory = std::path::PathBuf::from(system_root).join("System32");
    if !directory.is_dir() { return Err(io::Error::other("system directory unavailable")); }
    let cwd = wide(directory.as_os_str());
    // AppContainer startup on Windows uses LOCALAPPDATA to locate its
    // package profile. An *empty* environment causes CreateProcessW to
    // fail with ERROR_ENVVAR_NOT_FOUND (203), even when a profile exists.
    // Explicitly pass only the two trusted OS bootstrap variables, not
    // PATH, HOME, secrets, or the caller's ambient environment.
    let mut env = Vec::<u16>::new();
    for key in ["LOCALAPPDATA", "SystemRoot"] {
        let value = std::env::var_os(key)
            .ok_or_else(|| io::Error::other("required Windows bootstrap environment missing"))?;
        let units: Vec<u16> = value.encode_wide().collect();
        if units.is_empty() || units.iter().any(|x| *x == 0) {
            return Err(io::Error::other("invalid Windows bootstrap environment"));
        }
        env.extend(OsStr::new(key).encode_wide());
        env.push(b'=' as u16);
        env.extend(units);
        env.push(0);
    }
    env.push(0);
    if env.len() > 32_768 {
        return Err(io::Error::other("Windows bootstrap environment exceeds limit"));
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo = STARTUPINFOW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin_read.raw();
    startup.StartupInfo.hStdOutput = stdout_write.raw();
    startup.StartupInfo.hStdError = stderr.raw();
    startup.lpAttributeList = attr_guard.0;
    let mut info = PROCESS_INFORMATION::default();
    let created = unsafe { CreateProcessW(
        program.as_ptr(), cmd.as_mut_ptr(), null(), null(), 1,
        CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
        env.as_ptr().cast(), cwd.as_ptr(),
        (&startup.StartupInfo as *const STARTUPINFOW), &mut info
    ) };
    if created == 0 { return Err(io::Error::other(format!("CreateProcessW: {}", fail()))); }
    let process = Owned::new(info.hProcess)?;
    let thread = Owned::new(info.hThread)?;
    if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
        unsafe { windows_sys::Win32::System::Threading::TerminateProcess(process.raw(), 1); }
        return Err(io::Error::other(format!("AssignProcessToJobObject: {}", fail())));
    }
    if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
        unsafe { TerminateJobObject(job.raw(), 1); }
        return Err(fail());
    }
    // The only child-inherited handles are read stdin, write stdout and NUL.
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr);
    drop(thread);
    Ok(WindowsChild {
        _job: job, process, stdin: Some(stdin_write.file()), stdout: Some(stdout_read.file()),
    })
}
#[cfg(debug_assertions)]
pub fn windows_denial_probe(
    settings: &WorkerSettings, read: &Path, write: &Path, port: u16
) -> Result<(), ClientError> {
    let _lock = pin_executable(settings)?;
    let args = vec![
        "--probe-windows-denials".to_string(),
        read.to_string_lossy().into_owned(),
        write.to_string_lossy().into_owned(),
        port.to_string(),
    ];
    let mut child = spawn(&settings.executable, &args).map_err(|e| {
        eprintln!("SHIVA_WIN_PROBE_LAUNCH_DIAGNOSTIC: {e}");
        ClientError::Io
    })?;
    drop(child.stdin.take());
    drop(child.stdout.take());
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|_| ClientError::Io)? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                eprintln!("SHIVA_WIN_PROBE_WORKER_EXIT: {status:?}");
                return Err(ClientError::WorkerFailed);
            },
            None if start.elapsed() >= Duration::from_secs(8) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ClientError::TimedOut);
            }
            None => sleep(Duration::from_millis(10)),
        }
    }
}
#[cfg(not(debug_assertions))]
pub fn windows_denial_probe(
    _: &WorkerSettings, _: &Path, _: &Path, _: u16
) -> Result<(), ClientError> {
    Err(ClientError::UnsupportedPlatform)
}
