#![cfg_attr(windows, windows_subsystem = "windows")]
// Stdio is explicitly redirected through inherited anonymous pipes;
// no Win32 console/window station is needed by the untrusted worker.
use std::io::{stdin, stdout};
fn main() {
    // Do not include attacker-controlled parser errors, paths or document text
    // in stderr. Exit without producing a misleading Complete outcome.
    std::panic::set_hook(Box::new(|_| eprintln!("SHIVA_WORKER_PARSER_ABORTED")));
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let probe = cfg!(all(debug_assertions, target_os="linux", target_arch="x86_64")) && args == ["--probe-confinement"];
    let win_probe = cfg!(all(debug_assertions, windows)) && args.len() == 5 && args[0] == "--probe-windows-denials";
    if !args.is_empty() && !probe && !win_probe {
        eprintln!("SHIVA_WORKER_UNSUPPORTED_ARGUMENTS");
        std::process::exit(2);
    }
    if shiva_worker::sandbox::confine().is_err() {
        eprintln!("SHIVA_WORKER_NO_OS_CONFINEMENT");
        std::process::exit(3);
    }
    #[cfg(all(debug_assertions, target_os = "linux", target_arch = "x86_64"))]
    if probe {
        if shiva_worker::sandbox::probe_denials() { return; }
        std::process::exit(4);
    }
    #[cfg(all(debug_assertions, windows))]
    if win_probe {
        let port = args[3].parse::<u16>().unwrap_or(0);
        if port != 0 && shiva_worker::sandbox::probe_windows_denials(&args[1], &args[2], port, &args[4]) {
            return;
        }
        std::process::exit(4);
    }
    let input = stdin();
    let output = stdout();
    if shiva_worker::run(input.lock(), output.lock()).is_err() {
        eprintln!("SHIVA_WORKER_REQUEST_FAILED");
        std::process::exit(5);
    }
}
