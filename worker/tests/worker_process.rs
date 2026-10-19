use serde::Serialize;
use shiva_extractor_protocol::{
    read_frame, ExtractedFrame, ExtractorRevision, Message, Outcome,
    SourceRevision, StreamValidator, PROTOCOL_VERSION,
};
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

#[derive(Serialize)]
struct Request { schema_version: u16, source: SourceRevision }

fn wire(bytes: &[u8], format: &str) -> (Vec<u8>, SourceRevision) {
    let source = SourceRevision {
        source_id: "host_authorized_7".into(),
        input_bytes: bytes.len() as u64,
        blake3_hex: blake3::hash(bytes).to_hex().to_string(),
        format: format.into(),
    };
    let header = serde_json::to_vec(&Request {
        schema_version: PROTOCOL_VERSION, source: source.clone()
    }).unwrap();
    let mut input = (header.len() as u32).to_le_bytes().to_vec();
    input.extend(header);
    input.extend(bytes);
    (input, source)
}

fn invoke(bytes: &[u8], format: &str) -> (std::process::Output, SourceRevision) {
    let (wire, source) = wire(bytes, format);
    let mut child = Command::new(env!("CARGO_BIN_EXE_shiva-worker"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(&wire).unwrap();
    (child.wait_with_output().unwrap(), source)
}

#[cfg(all(target_os="linux", target_arch="x86_64"))]
#[test]
fn real_os_filter_denies_files_writes_and_network() {
    let process = Command::new(env!("CARGO_BIN_EXE_shiva-worker"))
        .arg("--probe-confinement").output().unwrap();
    assert!(process.status.success(), "sandbox denial probe failed");
}

#[cfg(all(target_os="linux", target_arch="x86_64"))]
#[test]
fn subprocess_stream_matches_host_verified_hash_and_end() {
    let (process, source) = invoke("Привет, 世界!".as_bytes(), "txt");
    assert!(process.status.success(), "worker execution failed");
    let mut cursor = Cursor::new(&process.stdout);
    let mut checker = StreamValidator::new(source, ExtractorRevision {
        name:"shiva-worker".into(), version:"0.1.0".into()
    });
    let mut frames: Vec<ExtractedFrame> = Vec::new();
    while (cursor.position() as usize) < process.stdout.len() {
        let frame = read_frame(&mut cursor).unwrap();
        checker.accept(&frame).unwrap();
        frames.push(frame);
    }
    assert!(checker.is_finished());
    assert!(frames.iter().any(|f| matches!(f.message, Message::Batch{..})));
    assert!(matches!(frames.last().unwrap().message, Message::End{outcome:Outcome::Complete,..}));
}

#[cfg(all(target_os="linux", target_arch="x86_64"))]
#[test]
fn asset_urls_fail_closed_without_fallback_read() {
    let (process, source) = invoke(b"intro ![secret](/etc/passwd)", "md");
    assert!(process.status.success(), "a rejected asset must be a framed failure");
    let mut c = Cursor::new(&process.stdout);
    let terminal = read_frame(&mut c).unwrap();
    assert_eq!(terminal.source, source);
    assert!(matches!(terminal.message, Message::End{outcome:Outcome::Failed,..}));
    assert_eq!(c.position() as usize, process.stdout.len());
}

#[cfg(all(target_os="linux", target_arch="x86_64"))]
#[test]
fn unsupported_formats_never_masquerade_as_success() {
    let (process, _) = invoke(b"%PDF-1.7", "pdf");
    assert!(process.status.success());
    let frame = read_frame(&mut Cursor::new(&process.stdout)).unwrap();
    assert!(matches!(frame.message, Message::End{outcome:Outcome::Failed,..}));
}

#[cfg(all(target_os="linux", target_arch="x86_64"))]
#[test]
fn malformed_frames_produce_no_false_success() {
    let mut process = Command::new(env!("CARGO_BIN_EXE_shiva-worker"))
        .stdin(Stdio::piped()).stdout(Stdio::piped())
        .spawn().unwrap();
    process.stdin.take().unwrap().write_all(&(100_000u32).to_le_bytes()).unwrap();
    let result = process.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}
