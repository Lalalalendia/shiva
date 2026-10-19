#![cfg(windows)]
// Windows release-image admission negatives. This is a preflight test,
// NOT proof of immutable installation ACLs or signer provenance.
use shiva_worker_client::{binary_blake3, extract, ClientError, WorkerSettings};
use std::path::PathBuf;
use std::time::Duration;

fn worker() -> PathBuf {
    PathBuf::from(std::env::var_os("SHIVA_TEST_WORKER_BIN")
        .expect("native Windows CI must provide exact worker binary"))
}
fn settings() -> WorkerSettings {
    let executable = worker();
    WorkerSettings {
        expected_binary_blake3: binary_blake3(&executable).expect("hash real worker"),
        executable,
        expected_version: "0.1.0".to_owned(),
        timeout: Duration::from_secs(6),
    }
}

#[test]
fn junctioned_worker_ancestor_is_rejected_before_spawn() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let original = worker();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let fixture = std::env::temp_dir()
        .join(format!("shiva-image-identity-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&fixture).unwrap();
    let junction = fixture.join("fake-install");
    let original_dir = original.parent().expect("worker directory");
    // PowerShell's Junction item avoids cmd.exe /C quoting ambiguities
    // on Windows CI. Escape single quotes in the two literal path values.
    // Ordinary users may create a junction without symlink privilege.
    fn ps_literal(path: &std::path::Path) -> String {
        path.to_string_lossy().replace('\'', "''")
    }
    let command = format!(
        "New-Item -ItemType Junction -Path '{}' -Target '{}' -ErrorAction Stop | Out-Null",
        ps_literal(&junction), ps_literal(original_dir)
    );
    let created = std::process::Command::new("pwsh.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output().expect("invoke PowerShell for test junction");
    assert!(created.status.success(),
        "PowerShell Junction fixture failed: stdout={} stderr={}",
        String::from_utf8_lossy(&created.stdout),
        String::from_utf8_lossy(&created.stderr));
    let alias = junction.join(original.file_name().unwrap());
    assert!(alias.is_file(), "positive control: junction must resolve to the actual executable");
    let mut cfg = settings();
    cfg.executable = alias;
    let result = extract(&cfg, "junction_probe", "txt", b"never launch");
    assert!(matches!(result, Err(ClientError::InvalidConfiguration)),
        "redirected worker pathname was not rejected: {result:?}");
    std::fs::remove_dir(&junction).expect("unlink junction only");
    std::fs::remove_dir(&fixture).expect("remove fixture");
    eprintln!("SHIVA_WIN_REPARSE_ANCESTOR_DENIED=pass");
}

#[test]
fn network_and_parent_traversal_worker_paths_fail_closed() {
    let mut cfg = settings();
    cfg.executable = PathBuf::from(r"\\untrusted.invalid\share\shiva-worker.exe");
    assert!(matches!(
        extract(&cfg, "unc_probe", "txt", b"no network path"),
        Err(ClientError::InvalidConfiguration)
    ), "UNC executable paths may not initiate remote filesystem access");

    let original = worker();
    let mut cfg = settings();
    cfg.executable = original.parent().unwrap().join("..").join(
        original.file_name().unwrap()
    );
    assert!(matches!(
        extract(&cfg, "parent_probe", "txt", b"no path traversal"),
        Err(ClientError::InvalidConfiguration)
    ), "lexical parent traversal must be rejected before accessing disk");
    eprintln!("SHIVA_WIN_WORKER_PATH_DENIALS=pass");
}
