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
#[cfg(all(
    any(all(target_os="linux", target_arch="x86_64"), windows),
    not(all(feature = "qa-pdf-worker", debug_assertions, target_os = "linux", target_arch = "x86_64"))
))]
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
    let aap_only = std::path::PathBuf::from(
        std::env::var_os("SHIVA_TEST_AAP_ONLY_FILE")
            .expect("mandatory LPAC ALL APPLICATION PACKAGES denial fixture missing")
    );
    assert!(aap_only.is_file(), "AAP-only fixture missing");
    assert_eq!(std::fs::read(&aap_only).unwrap(),
        b"shiva-aap-canary", "host must be able to read AAP-only control");
    let config = settings(worker_path());
    let result = windows_denial_probe(&config, &sentinel, &attempted_write, port, &aap_only);
    assert!(result.is_ok(), "native AppContainer/Job deny probe failed: {result:?}");
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "PRIVATE-VALUE-DO-NOT-EXPOSE");
    assert!(!attempted_write.exists(), "restricted child wrote outside sandbox");
    std::fs::remove_file(&sentinel).unwrap();
    std::fs::remove_dir(&protected).unwrap();
}


/// Prove JOB_LIST works when the host itself already belongs to an outer Job.
/// Run the restricted host in its own test subprocess so it cannot contaminate
/// unrelated tests or inherit their process-wide Job membership.
#[cfg(windows)]
#[test]
fn nested_outer_job_allows_atomic_inner_worker_launch() {
    const CHILD: &str = "SHIVA_TEST_NESTED_JOB_HOST";
    if std::env::var_os(CHILD).is_none() {
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .arg("--exact")
            .arg("nested_outer_job_allows_atomic_inner_worker_launch")
            .arg("--nocapture")
            .env(CHILD, "1")
            .output()
            .expect("spawn isolated outer-job host");
        let child_stdout = String::from_utf8_lossy(&output.stdout);
        let child_stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(),
            "nested job subprocess failed: status={:?}; stdout={}; stderr={}",
            output.status, child_stdout, child_stderr);
        assert!(child_stderr.contains("SHIVA_WIN_NESTED_JOB_LIST=pass"),
            "nested worker did not emit required security proof: {child_stderr}");
        // The inner subprocess output is otherwise captured and not visible
        // to GitHub Actions. Emit the tested proof in the CI job log.
        eprintln!("SHIVA_WIN_NESTED_JOB_LIST=pass");
        return;
    }

    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    use std::ptr::null;

    struct OuterJob(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for OuterJob {
        fn drop(&mut self) { unsafe { CloseHandle(self.0); } }
    }
    let raw = unsafe { CreateJobObjectW(null(), null()) };
    assert!(!raw.is_null(), "CreateJobObjectW: {}", unsafe { GetLastError() });
    let outer = OuterJob(raw);
    let current = unsafe { GetCurrentProcess() };
    assert_ne!(unsafe { AssignProcessToJobObject(outer.0, current) }, 0,
        "outer Job assignment unavailable: {}", unsafe { GetLastError() });
    let mut member = 0i32;
    assert_ne!(unsafe { IsProcessInJob(current, outer.0, &mut member) }, 0);
    assert_eq!(member, 1, "test host is not in its outer Job");

    let cfg = settings(worker_path());
    let verified = extract(&cfg, "nested_job_doc", "txt", "Вложенная работа 世界".as_bytes())
        .expect("nested outer Job must not prevent contained Shiva extraction");
    assert!(verified.blocks.iter().any(|block| block.text.contains("世界")));
    eprintln!("SHIVA_WIN_NESTED_JOB_LIST=pass");
}


// QA-only: deterministic real PDF bytes, never a filesystem path/URL.
// This intentionally exercises the strict host -> OS-contained child ->
// framed result -> host's Complete-only transaction boundary.
#[cfg(all(feature = "qa-pdf-worker", debug_assertions, target_os = "linux", target_arch = "x86_64"))]
fn synthetic_pdf(lines: &[&str], with_graphics: bool) -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream, StringFormat};

    let mut pdf = Document::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let font_id = pdf.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = pdf.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let mut ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
        Operation::new("Td", vec![30.into(), 720.into()]),
    ];
    for (i, text) in lines.iter().enumerate() {
        if i > 0 {
            ops.push(Operation::new("Td", vec![0.into(), (-16).into()]));
        }
        ops.push(Operation::new(
            "Tj",
            vec![Object::String(text.as_bytes().to_vec(), StringFormat::Literal)],
        ));
    }
    ops.push(Operation::new("ET", vec![]));
    if with_graphics {
        // A real visible rectangle, not just suspicious PDF metadata.
        ops.push(Operation::new("re", vec![10.into(), 10.into(), 50.into(), 50.into()]));
        ops.push(Operation::new("f", vec![]));
    }
    let contents_id = pdf.add_object(Stream::new(
        dictionary! {},
        Content { operations: ops }.encode().expect("encode PDF QA operators"),
    ));
    let page_id = pdf.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Resources" => resources_id,
        "Contents" => contents_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
        "Type" => "Pages",
        "Count" => 1,
        "Kids" => vec![page_id.into()],
    }));
    let catalog_id = pdf.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    pdf.trailer.set("Root", catalog_id);
    let mut output = Vec::new();
    pdf.save_to(&mut output).expect("save QA-only PDF");
    assert!(output.starts_with(b"%PDF-") && output.len() < 128_000);
    output
}

#[cfg(all(feature = "qa-pdf-worker", debug_assertions, target_os = "linux", target_arch = "x86_64"))]
#[test]
fn real_confined_pdf_child_roundtrip_via_pinned_host_and_complete_only_commit() {
    let pdf = synthetic_pdf(&["AQPDFALPHA", "AQPDFBETA"], false);
    let authorized = settings(worker_path());
    let verified = extract(&authorized, "authorized_pdf_lab_1", "pdf", &pdf)
        .expect("QA-only isolated PDF must return a complete verified stream");
    assert_eq!(verified.source.blake3_hex, blake3::hash(&pdf).to_hex().to_string());
    assert_eq!(verified.source.input_bytes, pdf.len() as u64);
    assert_eq!(verified.source.source_id, "authorized_pdf_lab_1");
    let joined = verified.blocks.iter()
        .map(|block| block.text.as_str()).collect::<Vec<_>>().join("");
    let first = joined.find("AQPDFALPHA").expect("source marker A lost");
    let second = joined.find("AQPDFBETA").expect("source marker B lost");
    assert!(second > first, "source text order changed");
    assert_eq!(joined.matches("AQPDFALPHA").count(), 1);
    assert_eq!(joined.matches("AQPDFBETA").count(), 1);
    eprintln!("SHIVA_PDF_WORKER_QA=pass;source_bound=true;markers=2;os_contained=true");
}

#[cfg(all(feature = "qa-pdf-worker", debug_assertions, target_os = "linux", target_arch = "x86_64"))]
#[test]
fn incomplete_or_corrupt_pdf_cannot_reach_verified_document() {
    let cfg = settings(worker_path());
    let blank = synthetic_pdf(&[], false);
    assert!(matches!(
        extract(&cfg, "blank_pdf_qa", "pdf", &blank),
        Err(ClientError::FailedExtraction)
    ), "a PDF with no extracted text cannot be Complete");
    assert!(matches!(
        extract(&cfg, "bad_pdf_qa", "pdf", b"%PDF-1.7\nbroken"),
        Err(ClientError::FailedExtraction)
    ), "a malformed PDF cannot reach the indexable result");
    let with_graphics = synthetic_pdf(&["AQPDFALPHA"], true);
    assert!(matches!(
        extract(&cfg, "graphics_pdf_qa", "pdf", &with_graphics),
        Err(ClientError::FailedExtraction)
    ), "partial text plus vector drawing must not be reported Complete");
    assert!(matches!(
        extract(&cfg, "unsupported_docx_qa", "docx", b"PK\x03\x04"),
        Err(ClientError::InvalidFormat)
    ), "DOCX must remain disabled even in PDF QA mode");
}
