use std::io::{stdin, stdout};
fn main() {
    // Do not include attacker-controlled parser errors, paths or document text
    // in stderr. Exit without producing a misleading Complete outcome.
    std::panic::set_hook(Box::new(|_| eprintln!("SHIVA_WORKER_PARSER_ABORTED")));
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let probe = cfg!(debug_assertions) && args == ["--probe-confinement"];
    if !args.is_empty() && !probe {
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
    let input = stdin();
    let output = stdout();
    if shiva_worker::run(input.lock(), output.lock()).is_err() {
        eprintln!("SHIVA_WORKER_REQUEST_FAILED");
        std::process::exit(5);
    }
}
