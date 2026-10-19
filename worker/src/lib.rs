//! Byte-only, one-document Shiva worker. No path-taking APIs are exposed.
//! This is a Linux x86_64 research slice, not an Akula integration.
pub mod sandbox;

use bytes::Bytes;
use serde::Deserialize;
use shiva::core::{Document, DocumentType, Element, TransformerTrait, TransformerWithImageLoaderSaverTrait};
use shiva_extractor_protocol::{
    write_frame, BlockKind, ContentBlock, ExtractedFrame, ExtractionWarning, ExtractorRevision,
    Locator, Message, Outcome, SourceRevision, WarningCode, PROTOCOL_VERSION,
};
use std::io::{Read, Write};

pub const MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 4096;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_BLOCKS: usize = 2048;
const CHUNK_BYTES: usize = 4096;
const BATCH_BLOCKS: usize = 32;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: u16,
    source: SourceRevision,
}

fn result_identity() -> ExtractorRevision {
    ExtractorRevision {
        name: "shiva-worker".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn parse_request<R: Read>(reader: &mut R) -> Result<(SourceRevision, Vec<u8>), &'static str> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix).map_err(|_| "truncated request header")?;
    let size = u32::from_le_bytes(prefix) as usize;
    if size == 0 || size > MAX_REQUEST_BYTES {
        return Err("request header budget exceeded");
    }
    let mut header = vec![0u8; size];
    reader.read_exact(&mut header).map_err(|_| "truncated request header")?;
    let request: Request = serde_json::from_slice(&header).map_err(|_| "invalid request JSON")?;
    if request.schema_version != PROTOCOL_VERSION {
        return Err("unsupported request protocol version");
    }
    let source = request.source;
    if source.input_bytes > MAX_INPUT_BYTES {
        return Err("input budget exceeded");
    }
    if !source.source_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        || source.source_id.is_empty() || source.source_id.len() > 128
        || source.blake3_hex.len() != 64
        || !source.blake3_hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid source identity");
    }
    let mut bytes = vec![0u8; source.input_bytes as usize];
    reader.read_exact(&mut bytes).map_err(|_| "truncated document input")?;
    if blake3::hash(&bytes).to_hex().as_str() != source.blake3_hex {
        return Err("input content hash does not match revision");
    }
    Ok((source, bytes))
}

fn parse_document(format: &str, content: &[u8]) -> Result<Document, &'static str> {
    let source = Bytes::copy_from_slice(content);
    // No filesystem loader, network fetch, font resolution or user paths.
    // Unsupported embedded image references cause a FAILED terminal status.
    let deny_asset = |_: &str| -> anyhow::Result<Bytes> {
        anyhow::bail!("external assets are not allowed")
    };
    match format {
        "txt" => shiva::text::Transformer::parse(&source)
            .map_err(|_| "text parser rejected input"),
        "md" => shiva::markdown::Transformer::parse_with_loader(&source, deny_asset)
            .map_err(|_| "markdown parser rejected input or external asset"),
        "html" => shiva::html::Transformer::parse_with_loader(&source, deny_asset)
            .map_err(|_| "HTML parser rejected input or external asset"),
        _ => Err("unsupported document format"),
    }
}

#[derive(Default)]
struct Extracted {
    blocks: Vec<ContentBlock>,
    used_text_bytes: usize,
    truncated: bool,
}

impl Extracted {
    fn push(&mut self, kind: BlockKind, value: &str) {
        if self.truncated || value.trim().is_empty() {
            return;
        }
        let mut part = String::new();
        for ch in value.chars() {
            if part.len() + ch.len_utf8() > CHUNK_BYTES {
                self.push_part(kind, &part);
                part.clear();
                if self.truncated {
                    return;
                }
            }
            part.push(ch);
        }
        if !part.is_empty() {
            self.push_part(kind, &part);
        }
    }

    fn push_part(&mut self, kind: BlockKind, part: &str) {
        if self.truncated {
            return;
        }
        if self.blocks.len() >= MAX_BLOCKS
            || self.used_text_bytes.saturating_add(part.len()) > MAX_TEXT_BYTES
        {
            self.truncated = true;
            return;
        }
        self.used_text_bytes += part.len();
        self.blocks.push(ContentBlock {
            id: self.blocks.len() as u64,
            kind,
            text: part.to_owned(),
            locator: Locator::Unknown,
        });
    }

    fn walk(&mut self, item: &Element, depth: usize) {
        if self.truncated {
            return;
        }
        if depth > 64 {
            self.truncated = true;
            return;
        }
        match item {
            Element::Text { text, .. } => self.push(BlockKind::Body, text),
            Element::Header { text, .. } => self.push(BlockKind::Heading, text),
            Element::Paragraph { elements } => {
                for item in elements { self.walk(item, depth + 1); }
            }
            Element::List { elements, .. } => {
                for item in elements { self.walk(&item.element, depth + 1); }
            }
            Element::Table { headers, rows } => {
                for column in headers {
                    self.walk(&column.element, depth + 1);
                }
                for row in rows {
                    for cell in &row.cells {
                        self.walk(&cell.element, depth + 1);
                    }
                }
            }
            Element::Hyperlink { title, .. } => self.push(BlockKind::Body, title),
            // Asset bytes are intentionally not indexed or opened.
            Element::Image(_) => self.truncated = true,
        }
    }
}

fn frame(source: &SourceRevision, message: Message) -> ExtractedFrame {
    ExtractedFrame {
        schema_version: PROTOCOL_VERSION,
        source: source.clone(),
        extractor: result_identity(),
        message,
    }
}

fn failed<W: Write>(writer: &mut W, source: &SourceRevision, code: WarningCode, note: &str) -> Result<(), &'static str> {
    let terminal = frame(source, Message::End {
        sequence: 0,
        outcome: Outcome::Failed,
        warnings: vec![ExtractionWarning { code, note: note.to_owned() }],
    });
    write_frame(writer, &terminal).map_err(|_| "failed to write outcome")
}

/// Consume precisely one request from a bounded stdin stream. No filenames or
/// caller-selected file descriptors are accepted as request fields.
pub fn run<R: Read, W: Write>(mut input: R, mut output: W) -> Result<(), &'static str> {
    let (source, bytes) = parse_request(&mut input)?;
    if !matches!(source.format.as_str(), "txt" | "md" | "html") {
        return failed(&mut output, &source, WarningCode::Unsupported, "Format disabled by worker policy");
    }
    let document = match std::panic::catch_unwind(|| parse_document(&source.format, &bytes)) {
        Ok(Ok(document)) => document,
        _ => {
            return failed(&mut output, &source, WarningCode::Decode,
                "Parser rejected document or external resource");
        }
    };
    let mut extracted = Extracted::default();
    for item in document.get_all_elements() {
        extracted.walk(item, 0);
        if extracted.truncated { break; }
    }
    if extracted.blocks.is_empty() && !extracted.truncated {
        return failed(&mut output, &source, WarningCode::Decode, "No indexable text");
    }
    let mut sequence = 0u64;
    for batch in extracted.blocks.chunks(BATCH_BLOCKS) {
        let message = Message::Batch {
            sequence,
            blocks: batch.to_vec(),
        };
        write_frame(&mut output, &frame(&source, message))
            .map_err(|_| "failed to write extraction block")?;
        sequence += 1;
    }
    let (outcome, warnings) = if extracted.truncated {
        (
            Outcome::Partial,
            vec![ExtractionWarning {
                code: WarningCode::ResourceLimit,
                note: "Extraction budget or unsupported embedded resource".to_string(),
            }],
        )
    } else {
        (Outcome::Complete, Vec::new())
    };
    write_frame(&mut output, &frame(&source, Message::End {
        sequence, outcome, warnings,
    })).map_err(|_| "failed to write terminal status")?;
    output.flush().map_err(|_| "failed to flush output")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use shiva_extractor_protocol::{read_frame, StreamValidator};
    use std::io::Cursor;

    #[derive(Serialize)]
    struct TestRequest { schema_version: u16, source: SourceRevision }

    fn sample(content: &[u8], format: &str) -> Vec<u8> {
        let source = SourceRevision {
            source_id: "authorized_1".into(),
            blake3_hex: blake3::hash(content).to_hex().to_string(),
            format: format.into(),
            input_bytes: content.len() as u64,
        };
        let header = serde_json::to_vec(&TestRequest {
            schema_version: PROTOCOL_VERSION,
            source,
        }).unwrap();
        let mut wire = (header.len() as u32).to_le_bytes().to_vec();
        wire.extend(header);
        wire.extend(content);
        wire
    }

    fn verify(payload: &[u8], format: &str) -> Vec<ExtractedFrame> {
        let mut output = Vec::new();
        run(Cursor::new(sample(payload, format)), &mut output).unwrap();
        let mut input = Cursor::new(output);
        let mut frames = Vec::new();
        while (input.position() as usize) < input.get_ref().len() {
            frames.push(read_frame(&mut input).unwrap());
        }
        let source = SourceRevision {
            source_id: "authorized_1".into(),
            blake3_hex: blake3::hash(payload).to_hex().to_string(),
            format: format.into(),
            input_bytes: payload.len() as u64,
        };
        let mut v = StreamValidator::new(source, result_identity());
        for f in &frames { v.accept(f).unwrap(); }
        assert!(v.is_finished());
        frames
    }

    #[test]
    fn text_and_unicode_are_projected_without_paths() {
        let frames = verify("Привет\n世界".as_bytes(), "txt");
        assert!(frames.iter().any(|f| matches!(f.message, Message::Batch { .. })));
        assert!(matches!(frames.last().unwrap().message, Message::End { outcome: Outcome::Complete, .. }));
        let json = serde_json::to_string(&frames).unwrap();
        assert!(json.contains("Привет"));
        assert!(!json.contains("/tmp/"));
    }

    #[test]
    fn markdown_heading_and_body() {
        let frames = verify(b"# Heading\n\ntext", "md");
        assert!(frames.iter().any(|f| matches!(f.message, Message::Batch { .. })));
        assert!(matches!(frames.last().unwrap().message, Message::End { outcome: Outcome::Complete, .. }));
    }

    #[test]
    fn html_plain_text_is_allowed() {
        let frames = verify(b"<h1>Hello</h1><p>World</p>", "html");
        assert!(matches!(frames.last().unwrap().message, Message::End { outcome: Outcome::Complete, .. }));
    }

    #[test]
    fn remote_markdown_image_cannot_trigger_an_asset_loader() {
        let frames = verify(b"before ![alt](https://example.org/p.png)", "md");
        assert!(matches!(frames.last().unwrap().message, Message::End { outcome: Outcome::Failed, .. }));
        assert_eq!(frames.len(), 1, "must never emit a successful batch on error");
    }

    #[test]
    fn unsupported_pdf_fails_closed() {
        let frames = verify(b"%PDF-1.7", "pdf");
        assert!(matches!(frames.last().unwrap().message, Message::End { outcome: Outcome::Failed, .. }));
    }

    #[test]
    fn rejects_invalid_hash_and_oversized_header_before_parsing() {
        let mut wire = sample(b"safe", "txt");
        let offset = 4;
        let header_len = u32::from_le_bytes(wire[0..4].try_into().unwrap()) as usize;
        let head = std::str::from_utf8(&wire[offset..offset+header_len]).unwrap();
        let wrong = head.replace(&blake3::hash(b"safe").to_hex().to_string(), &"a".repeat(64));
        assert_eq!(head.len(), wrong.len());
        wire.splice(offset..offset+header_len, wrong.bytes());
        assert!(run(Cursor::new(wire), Vec::new()).is_err());
        let header = ((MAX_REQUEST_BYTES+1) as u32).to_le_bytes();
        assert!(run(Cursor::new(header), Vec::new()).is_err());
    }

    #[test]
    fn large_unicode_text_is_split_into_bounded_blocks() {
        let content = "漢".repeat(3000);
        let frames = verify(content.as_bytes(), "txt");
        for f in frames {
            if let Message::Batch { blocks, .. } = f.message {
                for b in blocks { assert!(b.text.len() <= CHUNK_BYTES); }
            }
        }
    }
}
