use shiva_worker_client::{binary_blake3, extract, ClientError, WorkerSettings};
use std::path::PathBuf;
use std::time::Duration;

fn worker_path() -> PathBuf {
    PathBuf::from(std::env::var("SHIVA_TEST_WORKER_BIN")
        .expect("CI must build shiva-worker before testing shiva-worker-client"))
}
fn settings(worker: PathBuf) -> WorkerSettings {
    WorkerSettings {
        expected_binary_blake3: binary_blake3(&worker).unwrap(),
        executable: worker,
        expected_version: "0.1.0".to_owned(),
        timeout: Duration::from_secs(6),
    }
}
#[cfg(any(all(target_os="linux",target_arch="x86_64"), windows))]
#[test]
fn exact_pinned_binary_produces_only_valid_complete_blocks() {
    let cfg = settings(worker_path());
    let verified = extract(&cfg, "note_1", "txt", "Привет 世界".as_bytes()).unwrap();
    assert_eq!(verified.source.source_id, "note_1");
    assert_eq!(verified.extractor.name, "shiva-worker");
    assert!(verified.blocks.iter().any(|b| b.text.contains("Привет")));
}
#[cfg(any(all(target_os="linux",target_arch="x86_64"), windows))]
#[test]
fn binary_hash_mismatch_is_rejected_before_spawn() {
    let mut cfg = settings(worker_path());
    cfg.expected_binary_blake3 = "f".repeat(64);
    assert!(matches!(extract(&cfg, "doc2", "txt", b"test"), Err(ClientError::WorkerHashMismatch)));
}
#[cfg(any(all(target_os="linux",target_arch="x86_64"), windows))]
#[test]
fn document_embedded_path_does_not_get_read_or_indexed() {
    let cfg = settings(worker_path());
    assert!(matches!(
        extract(&cfg, "doc3", "md", b"![secret](/etc/passwd)"),
        Err(ClientError::FailedExtraction)
    ));
}
#[cfg(any(all(target_os="linux",target_arch="x86_64"), windows))]
#[test]
fn unsupported_pdf_is_not_launched() {
    let cfg = settings(worker_path());
    assert!(matches!(
        extract(&cfg, "doc4", "pdf", b"%PDF-1.7"),
        Err(ClientError::InvalidFormat)
    ));
}
#[cfg(all(target_os="linux",target_arch="x86_64"))]
#[test]
fn host_deadline_kills_unresponsive_pinned_child() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("shiva-stalled-worker-{}-{nonce}",std::process::id()));
    // Explicit absolute exec avoids relying on PATH after env_clear().
    std::fs::write(&path, b"#!/bin/sh\nexec /bin/sleep 10\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut cfg = settings(path.clone());
    cfg.timeout = Duration::from_millis(250);
    let started = Instant::now();
    let result = extract(&cfg, "doc5", "txt", b"hello");
    let _ = std::fs::remove_file(&path);
    assert!(matches!(result, Err(ClientError::TimedOut)), "must kill a stalled worker");
    assert!(started.elapsed() < Duration::from_secs(3), "host must not wait for child sleep");
}

#[cfg(all(windows, debug_assertions))]
#[test]
fn normal_host_fails_child_containment_attestation() {
    assert!(shiva_worker_client::normal_host_rejected_by_attestation(),
        "host attestation accepted an ordinary unrestricted process");
}

#[cfg(windows)]
#[test]
fn appcontainer_denies_real_vault_file_write_network_and_child_process() {
    use shiva_worker_client::windows_denial_probe;
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let protected = std::env::temp_dir()
        .join(format!("shiva-win-protected-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&protected).unwrap();
    let sentinel = protected.join("vault-secret.txt");
    let attempted_write = protected.join("escape-write.txt");
    std::fs::write(&sentinel, "PRIVATE-VALUE-DO-NOT-EXPOSE").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Positive control: the same host process must be able to connect to
    // the exact endpoint. Otherwise a blocked-worker result is meaningless.
    let reachable = std::net::TcpStream::connect(("127.0.0.1", port))
        .expect("host loopback positive control failed");
    let (_accepted, _) = listener.accept().unwrap();
    drop(reachable);
    let system_root = std::env::var_os("SystemRoot").expect("missing host SystemRoot");
    let cmd = std::path::PathBuf::from(system_root).join("System32").join("cmd.exe");
    let control_child = std::process::Command::new(&cmd)
        .arg("/C").arg("exit 0").status()
        .expect("host must be able to start known system executable");
    assert!(control_child.success(), "host child-process positive control failed");
    let config = settings(worker_path());
    let result = windows_denial_probe(&config, &sentinel, &attempted_write, port);
    assert!(result.is_ok(), "native AppContainer/Job deny probe failed: {result:?}");
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "PRIVATE-VALUE-DO-NOT-EXPOSE");
    assert!(!attempted_write.exists(), "restricted child wrote outside sandbox");
    std::fs::remove_file(&sentinel).unwrap();
    std::fs::remove_dir(&protected).unwrap();
}
