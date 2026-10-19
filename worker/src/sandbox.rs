//! The Linux x86_64 preview installs a fail-closed syscall allowlist before
//! deserializing any untrusted input. Other targets refuse to execute.
//!
//! The caller must still apply independent process supervision, binary pinning,
//! deadlines, and an external kill-on-timeout policy.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn confine() -> Result<(), &'static str> {
    // SECCOMP_RET_ERRNO(EPERM) for denied syscalls; unknown syscalls are not
    // implicitly permitted. No open/openat/openat2, sockets, exec or fork.
    const AUDIT_ARCH_X86_64: u32 = 0xc000003e;
    const ALLOW: u32 = 0x7fff0000;
    const KILL: u32 = 0x80000000;
    const DENY: u32 = 0x0005_0000 | (libc::EPERM as u32);

    fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }

    unsafe {
        // Close all inherited descriptors other than stdin, stdout, stderr.
        // Refuse execution if the kernel cannot guarantee descriptor closure.
        if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) != 0 {
            return Err("close_range unavailable");
        }
        const MEMORY: libc::rlim_t = 512 * 1024 * 1024;
        let limits = [
            (libc::RLIMIT_AS, MEMORY),
            (libc::RLIMIT_CPU, 5 as libc::rlim_t),
            (libc::RLIMIT_FSIZE, 0 as libc::rlim_t),
            (libc::RLIMIT_CORE, 0 as libc::rlim_t),
            (libc::RLIMIT_NOFILE, 16 as libc::rlim_t),
        ];
        for (resource, upper) in limits {
            let value = libc::rlimit { rlim_cur: upper, rlim_max: upper };
            if libc::setrlimit(resource, &value) != 0 {
                return Err("could not apply resource limits");
            }
        }
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err("no_new_privs unavailable");
        }

        let mut code = vec![
            // Load seccomp_data.arch (offset 4); kill unexpected ABI.
            instruction((libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16, 0, 0, 4),
            instruction((libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16, 1, 0, AUDIT_ARCH_X86_64),
            instruction((libc::BPF_RET | libc::BPF_K) as u16, 0, 0, KILL),
            // Load seccomp_data.nr (offset 0).
            instruction((libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16, 0, 0, 0),
        ];
        let allowed: &[libc::c_long] = &[
            libc::SYS_read, libc::SYS_write, libc::SYS_readv, libc::SYS_writev,
            libc::SYS_close, libc::SYS_fstat, libc::SYS_lseek,
            libc::SYS_mmap, libc::SYS_munmap, libc::SYS_mremap, libc::SYS_mprotect,
            libc::SYS_brk, libc::SYS_madvise,
            libc::SYS_rt_sigaction, libc::SYS_rt_sigprocmask,
            libc::SYS_rt_sigreturn, libc::SYS_sigaltstack,
            libc::SYS_futex, libc::SYS_clock_gettime, libc::SYS_gettimeofday,
            libc::SYS_getrandom, libc::SYS_getpid, libc::SYS_gettid,
            libc::SYS_sched_yield, libc::SYS_restart_syscall, libc::SYS_exit,
            libc::SYS_exit_group,
        ];
        for syscall in allowed {
            code.push(instruction((libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16, 0, 1, *syscall as u32));
            code.push(instruction((libc::BPF_RET | libc::BPF_K) as u16, 0, 0, ALLOW));
        }
        code.push(instruction((libc::BPF_RET | libc::BPF_K) as u16, 0, 0, DENY));
        let mut prog = libc::sock_fprog {
            len: code.len() as u16,
            filter: code.as_mut_ptr(),
        };
        if libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &mut prog) != 0 {
            return Err("seccomp filter unavailable");
        }
    }
    Ok(())
}

#[cfg(not(any(all(target_os = "linux", target_arch = "x86_64"), windows)))]
pub fn confine() -> Result<(), &'static str> {
    Err("unsupported host: worker refuses execution without an OS sandbox")
}

#[cfg(windows)]
pub fn confine() -> Result<(), &'static str> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenIsAppContainer, TokenCapabilities, TOKEN_QUERY};
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use std::ptr::null_mut;
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("cannot verify worker token");
        }
        struct TokenHandle(windows_sys::Win32::Foundation::HANDLE);
        impl Drop for TokenHandle {
            fn drop(&mut self) { unsafe { CloseHandle(self.0); } }
        }
        let token = TokenHandle(token);
        let mut app = 0u32;
        let mut returned = 0u32;
        if GetTokenInformation(
            token.0, TokenIsAppContainer, (&mut app as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32, &mut returned
        ) == 0 || app != 1 {
            return Err("worker token is not an AppContainer");
        }
        // A process with capabilities could have network or file authority.
        let mut cap_words = [0usize; 32];
        if GetTokenInformation(
            token.0, TokenCapabilities, cap_words.as_mut_ptr().cast(),
            std::mem::size_of_val(&cap_words) as u32, &mut returned
        ) == 0 || (cap_words.as_ptr() as *const u32).read() != 0 {
            return Err("worker AppContainer has unexpected capabilities");
        }
        let mut in_job = 0i32;
        if IsProcessInJob(GetCurrentProcess(), null_mut(), &mut in_job) == 0 || in_job != 1 {
            return Err("worker is not inside a Job Object");
        }
    }
    Ok(())
}

#[cfg(all(debug_assertions, windows))]
pub fn probe_windows_denials(read: &str, write: &str, port: u16) -> bool {
    use std::fs::{File, OpenOptions};
    use std::net::{SocketAddr, TcpStream};
    use std::process::Command;
    use std::time::Duration;
    use std::io::Write;
    let denied_read = File::open(read).err().and_then(|e| e.raw_os_error()) == Some(5);
    let denied_write = OpenOptions::new().write(true).create_new(true).open(write)
        .err().and_then(|e| e.raw_os_error()) == Some(5);
    // WFP/AppContainer may silently drop a prohibited connection instead of
    // immediately reporting WSAEACCES. Bound the probe independently.
    // The trusted host positively verifies this loopback listener first.
    let target = SocketAddr::from(([127, 0, 0, 1], port));
    let denied_network = match TcpStream::connect_timeout(&target, Duration::from_millis(400)) {
        Ok(_) => false,
        Err(e) => matches!(e.raw_os_error(), Some(10013 | 10060 | 10051 | 10050))
            || e.kind() == std::io::ErrorKind::TimedOut,
    };
    // ActiveProcessLimit=1, plus AppContainer and child-process policy.
    let cmd = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
        .join("System32").join("cmd.exe");
    let denied_child = Command::new(cmd).arg("/C").arg("exit 0").status()
        .map(|status| !status.success()).unwrap_or(true);
    // Debug-only flags contain no paths, document content or credentials.
    // The parent captures these only for an E2E containment audit.
    let _ = writeln!(std::io::stdout(), "SHIVA_WIN_DENIAL_FLAGS={denied_read},{denied_write},{denied_network},{denied_child}");
    denied_read && denied_write && denied_network && denied_child
}

/// This executable's probe is debug-only. A success code means denial checks
/// ran *after* the seccomp filter was installed, not merely that a mock passed.
#[cfg(all(debug_assertions, target_os = "linux", target_arch = "x86_64"))]
pub fn probe_denials() -> bool {
    use std::fs::{File, OpenOptions};
    use std::net::TcpStream;
    let read = File::open("/etc/passwd");
    let write = OpenOptions::new().write(true).create(true).open("/tmp/shiva-worker-probe-denied");
    let socket = TcpStream::connect("127.0.0.1:9");
    [read.err(), write.err(), socket.err()]
        .iter()
        .all(|err| err.as_ref().and_then(|e| e.raw_os_error()) == Some(libc::EPERM))
}
