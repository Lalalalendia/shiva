//! Bounded, versioned *output* frames for an untrusted document extractor.
//!
//! This crate does not parse documents, open paths, start processes or grant
//! permissions. A host must verify source bytes, enforce OS sandboxing and
//! commit extracted data to its search index only after a validated outcome.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Read, Write};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_BLOCKS_PER_FRAME: usize = 128;
pub const MAX_BLOCK_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_TEXT_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_WARNINGS: usize = 32;

#[derive(Debug)]
pub enum ProtocolError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(&'static str),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "frame I/O: {err}"),
            Self::Json(err) => write!(f, "frame JSON: {err}"),
            Self::Invalid(reason) => write!(f, "invalid extractor frame: {reason}"),
        }
    }
}
impl std::error::Error for ProtocolError {}
impl From<std::io::Error> for ProtocolError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for ProtocolError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRevision {
    /// Opaque host-assigned key, never a local path.
    pub source_id: String,
    /// BLAKE3 of the input snapshot, independently computed by the host.
    pub blake3_hex: String,
    pub input_bytes: u64,
    pub format: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractorRevision {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedFrame {
    pub schema_version: u16,
    pub source: SourceRevision,
    pub extractor: ExtractorRevision,
    pub message: Message,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Message {
    Batch {
        sequence: u64,
        blocks: Vec<ContentBlock>,
    },
    End {
        sequence: u64,
        outcome: Outcome,
        warnings: Vec<ExtractionWarning>,
    },
}

impl Message {
    fn sequence(&self) -> u64 {
        match self {
            Self::Batch { sequence, .. } | Self::End { sequence, .. } => *sequence,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Complete,
    Partial,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionWarning {
    pub code: WarningCode,
    /// Short diagnostic category, never file paths or full document text.
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    Unsupported,
    Decode,
    Truncated,
    ResourceLimit,
    MissingResource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentBlock {
    /// Strictly increasing over the entire response stream.
    pub id: u64,
    pub kind: BlockKind,
    pub text: String,
    pub locator: Locator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Heading,
    Body,
    TableCell,
    Metadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Locator {
    Unknown,
    Page { zero_based_index: u32 },
    Sheet {
        sheet_name: String,
        row_index: u32,
        column_index: u32,
    },
    Section { zero_based_path: Vec<u32> },
}

fn validate_frame(frame: &ExtractedFrame) -> Result<(), ProtocolError> {
    if frame.schema_version != PROTOCOL_VERSION {
        return Err(ProtocolError::Invalid("unsupported schema version"));
    }
    let source = &frame.source;
    if source.source_id.is_empty()
        || source.source_id.len() > 128
        || !source
            .source_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(ProtocolError::Invalid("source id must be an opaque token"));
    }
    if source.blake3_hex.len() != 64
        || !source
            .blake3_hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ProtocolError::Invalid("invalid source BLAKE3"));
    }
    if source.format.is_empty()
        || source.format.len() > 24
        || !source.format.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(ProtocolError::Invalid("invalid detected format"));
    }
    if frame.extractor.name.is_empty()
        || frame.extractor.name.len() > 80
        || frame.extractor.version.is_empty()
        || frame.extractor.version.len() > 128
        || frame
            .extractor
            .name
            .chars()
            .chain(frame.extractor.version.chars())
            .any(char::is_control)
    {
        return Err(ProtocolError::Invalid("invalid extractor identity"));
    }

    match &frame.message {
        Message::Batch { blocks, .. } => {
            if blocks.is_empty() || blocks.len() > MAX_BLOCKS_PER_FRAME {
                return Err(ProtocolError::Invalid("invalid block count"));
            }
            let mut last_id = None;
            for block in blocks {
                if block.text.is_empty() || block.text.len() > MAX_BLOCK_TEXT_BYTES {
                    return Err(ProtocolError::Invalid("invalid block text length"));
                }
                if let Some(previous) = last_id {
                    if block.id <= previous {
                        return Err(ProtocolError::Invalid("block ids not increasing"));
                    }
                }
                match &block.locator {
                    Locator::Sheet { sheet_name, .. }
                        if sheet_name.is_empty() || sheet_name.len() > 256 =>
                    {
                        return Err(ProtocolError::Invalid("invalid sheet name"));
                    }
                    Locator::Section { zero_based_path } if zero_based_path.len() > 32 => {
                        return Err(ProtocolError::Invalid("locator path too deep"));
                    }
                    _ => {}
                }
                last_id = Some(block.id);
            }
        }
        Message::End {
            outcome, warnings, ..
        } => {
            if warnings.len() > MAX_WARNINGS {
                return Err(ProtocolError::Invalid("too many warnings"));
            }
            if *outcome != Outcome::Complete && warnings.is_empty() {
                return Err(ProtocolError::Invalid("incomplete extraction requires a warning"));
            }
            for warning in warnings {
                if warning.note.is_empty()
                    || warning.note.len() > 512
                    || warning.note.chars().any(char::is_control)
                {
                    return Err(ProtocolError::Invalid("invalid warning note"));
                }
            }
        }
    }
    Ok(())
}

/// Write a single length-prefixed JSON frame. The limit also bounds decoded
/// allocations on the host; a host must still cap the total stream and time.
pub fn write_frame<W: Write>(writer: &mut W, frame: &ExtractedFrame) -> Result<(), ProtocolError> {
    validate_frame(frame)?;
    let serialized = serde_json::to_vec(frame)?;
    if serialized.is_empty() || serialized.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::Invalid("serialized frame exceeds budget"));
    }
    writer.write_all(&(serialized.len() as u32).to_le_bytes())?;
    writer.write_all(&serialized)?;
    Ok(())
}

/// Reject oversized lengths *before* allocating the frame body.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<ExtractedFrame, ProtocolError> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix)?;
    let size = u32::from_le_bytes(prefix) as usize;
    if size == 0 || size > MAX_FRAME_BYTES {
        return Err(ProtocolError::Invalid("frame length exceeds budget"));
    }
    let mut bytes = vec![0u8; size];
    reader.read_exact(&mut bytes)?;
    let frame: ExtractedFrame = serde_json::from_slice(&bytes)?;
    validate_frame(&frame)?;
    Ok(frame)
}

/// Host-side state machine. This verifies ordering and source identity but
/// cannot grant filesystem access or attest that the worker parsed truthfully.
pub struct StreamValidator {
    expected_source: SourceRevision,
    expected_extractor: ExtractorRevision,
    next_sequence: u64,
    last_block_id: Option<u64>,
    seen_blocks: u64,
    total_text_bytes: u64,
    finished: bool,
}

impl StreamValidator {
    pub fn new(source: SourceRevision, extractor: ExtractorRevision) -> Self {
        Self {
            expected_source: source,
            expected_extractor: extractor,
            next_sequence: 0,
            last_block_id: None,
            seen_blocks: 0,
            total_text_bytes: 0,
            finished: false,
        }
    }

    pub fn accept(&mut self, frame: &ExtractedFrame) -> Result<(), ProtocolError> {
        validate_frame(frame)?;
        if self.finished {
            return Err(ProtocolError::Invalid("frames after terminal outcome"));
        }
        if frame.source != self.expected_source || frame.extractor != self.expected_extractor {
            return Err(ProtocolError::Invalid("source or extractor changed"));
        }
        if frame.message.sequence() != self.next_sequence {
            return Err(ProtocolError::Invalid("incorrect sequence number"));
        }
        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(ProtocolError::Invalid("sequence overflow"))?;

        match &frame.message {
            Message::Batch { blocks, .. } => {
                let mut last_id = self.last_block_id;
                let mut total_text_bytes = self.total_text_bytes;
                for block in blocks {
                    if let Some(previous) = last_id {
                        if block.id <= previous {
                            return Err(ProtocolError::Invalid("repeated or reordered block"));
                        }
                    }
                    total_text_bytes = total_text_bytes
                        .checked_add(block.text.len() as u64)
                        .ok_or(ProtocolError::Invalid("total text overflow"))?;
                    if total_text_bytes > MAX_TOTAL_TEXT_BYTES {
                        return Err(ProtocolError::Invalid("total extracted text exceeds budget"));
                    }
                    last_id = Some(block.id);
                }
                self.last_block_id = last_id;
                self.total_text_bytes = total_text_bytes;
                self.seen_blocks = self
                    .seen_blocks
                    .checked_add(blocks.len() as u64)
                    .ok_or(ProtocolError::Invalid("block count overflow"))?;
            }
            Message::End { outcome, .. } => {
                if *outcome == Outcome::Complete
                    && self.expected_source.input_bytes > 0
                    && self.seen_blocks == 0
                {
                    return Err(ProtocolError::Invalid(
                        "nonempty input cannot report an empty complete extraction",
                    ));
                }
                self.finished = true;
            }
        }
        self.next_sequence = next_sequence;
        Ok(())
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn total_blocks(&self) -> u64 {
        self.seen_blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn bindings() -> (SourceRevision, ExtractorRevision) {
        (
            SourceRevision {
                source_id: "note_123".into(),
                blake3_hex: "a".repeat(64),
                input_bytes: 14,
                format: "pdf".into(),
            },
            ExtractorRevision {
                name: "shiva".into(),
                version: "1.4.9-fork".into(),
            },
        )
    }

    fn frame(message: Message) -> ExtractedFrame {
        let (source, extractor) = bindings();
        ExtractedFrame {
            schema_version: PROTOCOL_VERSION,
            source,
            extractor,
            message,
        }
    }

    #[test]
    fn multilingual_round_trip_and_ordered_stream() {
        let batch = frame(Message::Batch {
            sequence: 0,
            blocks: vec![ContentBlock {
                id: 10,
                kind: BlockKind::Body,
                text: "Привет 世界 hello".into(),
                locator: Locator::Page {
                    zero_based_index: 7,
                },
            }],
        });
        let end = frame(Message::End {
            sequence: 1,
            outcome: Outcome::Complete,
            warnings: vec![],
        });
        let mut wire = Vec::new();
        write_frame(&mut wire, &batch).unwrap();
        write_frame(&mut wire, &end).unwrap();
        let mut cursor = Cursor::new(wire);
        let parsed = read_frame(&mut cursor).unwrap();
        let parsed_end = read_frame(&mut cursor).unwrap();
        assert_eq!(parsed, batch);
        assert_eq!(parsed_end, end);
        let (source, extractor) = bindings();
        let mut validator = StreamValidator::new(source, extractor);
        validator.accept(&parsed).unwrap();
        validator.accept(&parsed_end).unwrap();
        assert_eq!(validator.total_blocks(), 1);
        assert!(validator.is_finished());
        assert!(validator.accept(&parsed_end).is_err());
    }

    #[test]
    fn reject_oversized_prefix_without_allocating_payload() {
        let mut cursor = Cursor::new(((MAX_FRAME_BYTES + 1) as u32).to_le_bytes());
        assert!(matches!(
            read_frame(&mut cursor),
            Err(ProtocolError::Invalid("frame length exceeds budget"))
        ));
    }

    #[test]
    fn reject_version_source_swap_and_out_of_order_blocks() {
        let (source, extractor) = bindings();
        let mut validator = StreamValidator::new(source, extractor);
        let mut wrong = frame(Message::Batch {
            sequence: 1,
            blocks: vec![ContentBlock {
                id: 2,
                kind: BlockKind::Heading,
                text: "title".into(),
                locator: Locator::Unknown,
            }],
        });
        assert!(validator.accept(&wrong).is_err());
        wrong.schema_version = 2;
        assert!(validator.accept(&wrong).is_err());
        wrong.schema_version = PROTOCOL_VERSION;
        wrong.source.blake3_hex = "b".repeat(64);
        assert!(validator.accept(&wrong).is_err());
        wrong.source.blake3_hex = "a".repeat(64);
        if let Message::Batch { sequence, .. } = &mut wrong.message {
            *sequence = 0;
        }
        validator.accept(&wrong).unwrap();
        let duplicate = frame(Message::Batch {
            sequence: 1,
            blocks: vec![ContentBlock {
                id: 2,
                kind: BlockKind::Body,
                text: "duplicate".into(),
                locator: Locator::Unknown,
            }],
        });
        assert!(validator.accept(&duplicate).is_err());
        assert!(!validator.is_finished());
    }

    #[test]
    fn partial_and_failed_must_not_masquerade_as_complete() {
        let (source, extractor) = bindings();
        let mut validator = StreamValidator::new(source, extractor);
        let silent = frame(Message::End {
            sequence: 0,
            outcome: Outcome::Complete,
            warnings: vec![],
        });
        assert!(validator.accept(&silent).is_err());
        let missing_warning = frame(Message::End {
            sequence: 0,
            outcome: Outcome::Partial,
            warnings: vec![],
        });
        assert!(validator.accept(&missing_warning).is_err());
        let partial = frame(Message::End {
            sequence: 0,
            outcome: Outcome::Partial,
            warnings: vec![ExtractionWarning {
                code: WarningCode::Unsupported,
                note: "No text layer".into(),
            }],
        });
        validator.accept(&partial).unwrap();
        assert!(validator.is_finished());
    }

    #[test]
    fn reject_paths_as_opaque_source_ids_and_extra_json_fields() {
        let mut unexpected = frame(Message::End {
            sequence: 0,
            outcome: Outcome::Failed,
            warnings: vec![ExtractionWarning {
                code: WarningCode::Decode,
                note: "Parse failed".into(),
            }],
        });
        unexpected.source.source_id = "../secrets".into();
        let mut out = Vec::new();
        assert!(write_frame(&mut out, &unexpected).is_err());
        let mut value = serde_json::to_value(frame(Message::End {
            sequence: 0,
            outcome: Outcome::Failed,
            warnings: vec![ExtractionWarning {
                code: WarningCode::Decode,
                note: "Bad bytes".into(),
            }],
        }))
        .unwrap();
        value["unexpected"] = serde_json::Value::Bool(true);
        let body = serde_json::to_vec(&value).unwrap();
        let mut wire = (body.len() as u32).to_le_bytes().to_vec();
        wire.extend(body);
        assert!(read_frame(&mut Cursor::new(wire)).is_err());
    }

    #[test]
    fn oversized_block_and_deep_locator_are_rejected() {
        let mut bad = frame(Message::Batch {
            sequence: 0,
            blocks: vec![ContentBlock {
                id: 0,
                kind: BlockKind::Body,
                text: "x".repeat(MAX_BLOCK_TEXT_BYTES + 1),
                locator: Locator::Unknown,
            }],
        });
        assert!(write_frame(&mut Vec::new(), &bad).is_err());
        if let Message::Batch { blocks, .. } = &mut bad.message {
            blocks[0].text = "ok".into();
            blocks[0].locator = Locator::Section {
                zero_based_path: vec![0; 33],
            };
        }
        assert!(write_frame(&mut Vec::new(), &bad).is_err());
    }
}
