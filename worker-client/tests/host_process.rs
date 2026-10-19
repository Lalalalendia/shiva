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
#[cfg(all(target_os="linux",target_arch="x86_64"))]
#[test]
fn exact_pinned_binary_produces_only_valid_complete_blocks() {
    let cfg = settings(worker_path());
    let verified = extract(&cfg, "note_1", "txt", "Привет 世界".as_bytes()).unwrap();
    assert_eq!(verified.source.source_id, "note_1");
    assert_eq!(verified.extractor.name, "shiva-worker");
    assert!(verified.blocks.iter().any(|b| b.text.contains("Привет")));
}
#[cfg(all(target_os="linux",target_arch="x86_64"))]
#[test]
fn binary_hash_mismatch_is_rejected_before_spawn() {
    let mut cfg = settings(worker_path());
    cfg.expected_binary_blake3 = "f".repeat(64);
    assert!(matches!(extract(&cfg, "doc2", "txt", b"test"), Err(ClientError::WorkerHashMismatch)));
}
#[cfg(all(target_os="linux",target_arch="x86_64"))]
#[test]
fn document_embedded_path_does_not_get_read_or_indexed() {
    let cfg = settings(worker_path());
    assert!(matches!(
        extract(&cfg, "doc3", "md", b"![secret](/etc/passwd)"),
        Err(ClientError::FailedExtraction)
    ));
}
#[cfg(all(target_os="linux",target_arch="x86_64"))]
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
