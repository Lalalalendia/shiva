//! Host-side, *staged-only* client for the standalone Shiva worker.
//! This crate does not grant vault access or index into Tantivy.
//! Linux and Windows preview. Windows uses a native AppContainer + Job launcher.

use serde::Serialize;
use shiva_extractor_protocol::{
    read_frame, BlockKind, ContentBlock, ExtractedFrame, ExtractorRevision,
    Message, Outcome, SourceRevision, StreamValidator, PROTOCOL_VERSION,
};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod windows_launch;
#[cfg(windows)]
pub use windows_launch::windows_denial_probe;

pub const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 4096;
const WORKER_NAME: &str = "shiva-worker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    UnsupportedPlatform,
    InvalidConfiguration,
    InvalidSource,
    InvalidFormat,
    WorkerHashMismatch,
    Io,
    WorkerFailed,
    TimedOut,
    TooMuchOutput,
    InvalidProtocol,
    MissingTerminal,
    PartialExtraction,
    FailedExtraction,
}

#[derive(Debug)]
pub struct VerifiedDocument {
    pub source: SourceRevision,
    pub extractor: ExtractorRevision,
    pub blocks: Vec<ContentBlock>,
}

#[derive(Debug, Clone)]
pub struct WorkerSettings {
    /// Trusted path to an immutable installed worker artifact, NEVER an input
    /// value taken from a user document or its metadata.
    pub executable: PathBuf,
    /// Full lowercase BLAKE3 of executable bytes provisioned out-of-band.
    pub expected_binary_blake3: String,
    /// Pinned worker version in protocol End and Batch frames.
    pub expected_version: String,
    pub timeout: Duration,
}

#[derive(Serialize)]
struct Request {
    schema_version: u16,
    source: SourceRevision,
}

/// Hash a host-selected binary before spawn. This is provenance checking
/// only: replacing the binary between hashing and spawn is still a TOCTOU
/// risk and MUST be closed by the packaging/launch authority.
pub fn binary_blake3(path: &Path) -> Result<String, ClientError> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| ClientError::Io)?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > MAX_BINARY_BYTES {
        return Err(ClientError::InvalidConfiguration);
    }
    let mut file = std::fs::File::open(path).map_err(|_| ClientError::Io)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 16 * 1024];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buf).map_err(|_| ClientError::Io)?;
        if count == 0 { break; }
        total = total.saturating_add(count as u64);
        if total > MAX_BINARY_BYTES {
            return Err(ClientError::InvalidConfiguration);
        }
        hasher.update(&buf[..count]);
    }
    if total != meta.len() {
        return Err(ClientError::InvalidConfiguration);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn valid_identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 &&
        s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn check_settings(settings: &WorkerSettings, source_id: &str, format: &str, input: &[u8])
    -> Result<(), ClientError>
{
    if !cfg!(any(all(target_os = "linux", target_arch = "x86_64"), windows)) {
        return Err(ClientError::UnsupportedPlatform);
    }
    if !valid_identifier(source_id) || input.len() > MAX_INPUT_BYTES {
        return Err(ClientError::InvalidSource);
    }
    if !matches!(format, "txt" | "md" | "html") {
        return Err(ClientError::InvalidFormat);
    }
    if settings.timeout < Duration::from_millis(50)
        || settings.timeout > Duration::from_secs(30)
        || settings.expected_version.is_empty()
        || settings.expected_version.len() > 128
        || settings.expected_binary_blake3.len() != 64
        || !settings.expected_binary_blake3.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ClientError::InvalidConfiguration);
    }
    Ok(())
}

fn validate_completed(raw: &[u8], source: SourceRevision, extractor: ExtractorRevision)
    -> Result<VerifiedDocument, ClientError>
{
    let mut validator = StreamValidator::new(source.clone(), extractor.clone());
    let mut cursor = Cursor::new(raw);
    let mut blocks = Vec::new();
    let mut outcome = None;
    while (cursor.position() as usize) < raw.len() {
        let frame: ExtractedFrame = read_frame(&mut cursor).map_err(|_| ClientError::InvalidProtocol)?;
        validator.accept(&frame).map_err(|_| ClientError::InvalidProtocol)?;
        match frame.message {
            Message::Batch { blocks: batch, .. } => blocks.extend(batch),
            Message::End { outcome: terminal, .. } => outcome = Some(terminal),
        }
    }
    if !validator.is_finished() {
        return Err(ClientError::MissingTerminal);
    }
    match outcome {
        Some(Outcome::Complete) => Ok(VerifiedDocument { source, extractor, blocks }),
        Some(Outcome::Partial) => Err(ClientError::PartialExtraction),
        Some(Outcome::Failed) => Err(ClientError::FailedExtraction),
        None => Err(ClientError::MissingTerminal),
    }
}

/// One document, one child process. Only return blocks after the full terminal
/// status, source binding, frame ordering, and success exit are all verified.
/// No index write is ever performed by this crate.
pub fn extract(
    settings: &WorkerSettings,
    source_id: &str,
    format: &str,
    input: &[u8],
) -> Result<VerifiedDocument, ClientError> {
    check_settings(settings, source_id, format, input)?;
    #[cfg(windows)]
    let _pinned_image = windows_launch::pin_executable(settings)?;
    #[cfg(not(windows))]
    if binary_blake3(&settings.executable)? != settings.expected_binary_blake3 {
        return Err(ClientError::WorkerHashMismatch);
    }
    let source = SourceRevision {
        source_id: source_id.to_owned(),
        blake3_hex: blake3::hash(input).to_hex().to_string(),
        input_bytes: input.len() as u64,
        format: format.to_owned(),
    };
    let extractor = ExtractorRevision {
        name: WORKER_NAME.to_owned(),
        version: settings.expected_version.clone(),
    };
    let header = serde_json::to_vec(&Request {
        schema_version: PROTOCOL_VERSION,
        source: source.clone(),
    }).map_err(|_| ClientError::InvalidConfiguration)?;
    if header.is_empty() || header.len() > MAX_REQUEST_BYTES {
        return Err(ClientError::InvalidConfiguration);
    }

    // Avoid environment leakage, user-controlled cwd and stdin inherited FDs.
    // The trusted worker is required to install OS confinement *before*
    // accepting request bytes. It independently closes FDs >=3.
    #[cfg(not(windows))]
    let mut child = Command::new(&settings.executable)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn().map_err(|_| ClientError::Io)?;
    #[cfg(windows)]
    let mut child = windows_launch::spawn(&settings.executable, &[])
        .map_err(|e| {
            #[cfg(debug_assertions)]
            eprintln!("SHIVA_WIN_LAUNCH_DIAGNOSTIC: {e}");
            ClientError::Io
        })?;
    let mut child_input = child.stdin.take().ok_or(ClientError::Io)?;
    let child_output = child.stdout.take().ok_or(ClientError::Io)?;
    let owned_input = input.to_vec();
    let writer = thread::spawn(move || -> std::io::Result<()> {
        child_input.write_all(&(header.len() as u32).to_le_bytes())?;
        child_input.write_all(&header)?;
        child_input.write_all(&owned_input)?;
        child_input.flush()
    });
    let reader = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut result = Vec::new();
        child_output.take((MAX_OUTPUT_BYTES + 1) as u64).read_to_end(&mut result)?;
        Ok(result)
    });

    let started = Instant::now();
    let mut timeout = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= settings.timeout => {
                timeout = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let input_result = writer.join().map_err(|_| ClientError::Io)?;
    let output_result = reader.join().map_err(|_| ClientError::Io)?;
    if timeout { return Err(ClientError::TimedOut); }
    let status = status.ok_or(ClientError::WorkerFailed)?;
    if !status.success() {
        return Err(ClientError::WorkerFailed);
    }
    input_result.map_err(|_| ClientError::Io)?;
    let output = output_result.map_err(|_| ClientError::Io)?;
    if output.len() > MAX_OUTPUT_BYTES { return Err(ClientError::TooMuchOutput); }
    validate_completed(&output, source, extractor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiva_extractor_protocol::{write_frame, ExtractionWarning, WarningCode};

    fn source() -> SourceRevision {
        SourceRevision {
            source_id: "safe-1".to_string(),
            blake3_hex: "b".repeat(64),
            input_bytes: 2,
            format: "txt".to_string(),
        }
    }
    fn extractor() -> ExtractorRevision {
        ExtractorRevision {
            name: WORKER_NAME.into(),
            version: "0.1.0".into(),
        }
    }
    #[test]
    fn empty_and_incomplete_stream_are_not_indexable() {
        assert!(matches!(
            validate_completed(&[], source(), extractor()),
            Err(ClientError::MissingTerminal)
        ));
    }
    #[test]
    fn partial_stream_can_never_be_committed() {
        let mut raw = Vec::new();
        write_frame(&mut raw, &ExtractedFrame {
            schema_version: PROTOCOL_VERSION,
            source: source(), extractor: extractor(),
            message: Message::End {
                sequence: 0,
                outcome: Outcome::Partial,
                warnings: vec![ExtractionWarning {
                    code: WarningCode::ResourceLimit,
                    note: "Truncated".into(),
                }],
            },
        }).unwrap();
        assert!(matches!(
            validate_completed(&raw, source(), extractor()),
            Err(ClientError::PartialExtraction)
        ));
    }
    #[test]
    fn frames_cannot_change_revision_or_exit_without_end() {
        let mut raw = Vec::new();
        write_frame(&mut raw, &ExtractedFrame {
            schema_version: PROTOCOL_VERSION,
            source: source(), extractor: extractor(),
            message: Message::Batch {
                sequence: 0,
                blocks: vec![ContentBlock {
                    id: 0,
                    kind: BlockKind::Body,
                    text: "hello".into(),
                    locator: shiva_extractor_protocol::Locator::Unknown,
                }]
            },
        }).unwrap();
        assert!(matches!(validate_completed(&raw, source(), extractor()), Err(ClientError::MissingTerminal)));
        let mut bad = extractor();
        bad.version = "forged".into();
        assert!(matches!(validate_completed(&raw, source(), bad), Err(ClientError::InvalidProtocol)));
    }
}
