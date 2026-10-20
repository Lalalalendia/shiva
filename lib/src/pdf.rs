use crate::core::Element::{List, Paragraph, Text};
use crate::core::{Document, Element, ListItem, ParserError, TransformerTrait};

use anyhow;
use bytes::Bytes;
use log::warn;
use lopdf::content::Content;
use lopdf::Encoding;
use lopdf::{Document as PdfDocument, LoadOptions, Object, ObjectId};
use std::collections::BTreeMap;
use std::io::Read;
use typst::{eval::Tracer, foundations::Smart};

pub struct Transformer;

// Standalone library QA limits, not permission to enable PDF in the worker.
// Per-stream and page limits are supplemented below with cumulative page,
// text, operation and element limits. Process CPU, actual RSS and all PDF
// loading allocations still require a separate OS sandbox release gate.
const MAX_PDF_INPUT: usize = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_STREAM: usize = 8 * 1024 * 1024;
const MAX_TOUNICODE_CMAP: usize = 1024 * 1024;
const MAX_PDF_PAGES: usize = 1024;

impl TransformerTrait for Transformer {
    fn parse(document: &Bytes) -> anyhow::Result<Document> {
        let mut elements: Vec<Element> = Vec::new();
        let mut budget = PdfBudget::default();
        anyhow::ensure!(
            document.len() <= MAX_PDF_INPUT,
            "PDF input exceeds standalone parser byte limit"
        );
        let pdf_document = PdfDocument::load_mem_with_options(
            document,
            LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED_STREAM),
        )?;
        anyhow::ensure!(
            pdf_document.get_pages().len() <= MAX_PDF_PAGES,
            "PDF page count exceeds standalone parser limit"
        );
        for (_page_number, page_id) in pdf_document.get_pages() {
            // get_page_content() already joins the page's /Contents streams.
            // Parsing it once PER stream repeats every page's text N times.
            parse_page(page_id, &pdf_document, &mut elements, &mut budget)?;
        }
        Ok(Document::new(elements))
    }
    fn generate(document: &Document) -> anyhow::Result<Bytes> {
        let (text, img_map) = crate::typst::generate_document(document)?;

        let world = crate::typst::ShivaWorld::new(text, img_map)?;
        let mut tracer = Tracer::default();

        // Document-controlled content must never panic the PDF conversion API.
        let document = typst::compile(&world, &mut tracer).map_err(|diagnostics| {
            let messages = diagnostics
                .iter()
                .take(3)
                .map(|diagnostic| diagnostic.message.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::anyhow!(
                "Typst compilation failed ({} diagnostics): {}",
                diagnostics.len(),
                messages
            )
        })?;
        let warnings = tracer.warnings();

        if !warnings.is_empty() {
            // Trowing any warnings if necessary
            for warn in warnings {
                warn!("Warning - {}", warn.message);
            }
        }

        // Converting to pdf then to bytes
        let pdf = typst_pdf::pdf(&document, Smart::Auto, None);

        let bytes = Bytes::from(pdf);
        Ok(bytes)
    }
}

// These are library QA admission ceilings, not process RSS/CPU guarantees.
const MAX_PDF_TOTAL_PAGE_CONTENT: usize = 16 * 1024 * 1024;
const MAX_PDF_TOTAL_TEXT: usize = 8 * 1024 * 1024;
const MAX_PDF_EMITTED_ELEMENTS: usize = 100_000;
const MAX_PDF_OPERATIONS: usize = 200_000;
const MAX_PDF_TEXT_ARRAY_DEPTH: usize = 16;

#[derive(Default)]
struct PdfBudget {
    page_bytes: usize,
    text_bytes: usize,
    emitted_elements: usize,
    operations: usize,
}

impl PdfBudget {
    fn charge(counter: &mut usize, bytes: usize, max: usize, label: &str) -> anyhow::Result<()> {
        let next = counter.checked_add(bytes)
            .ok_or_else(|| anyhow::anyhow!("{label} counter overflow"))?;
        anyhow::ensure!(next <= max, "{label} exceeded");
        *counter = next;
        Ok(())
    }

    fn page_content(&mut self, bytes: usize) -> anyhow::Result<()> {
        Self::charge(&mut self.page_bytes, bytes, MAX_PDF_TOTAL_PAGE_CONTENT,
            "PDF cumulative decoded page budget")
    }

    fn text(&mut self, bytes: usize) -> anyhow::Result<()> {
        Self::charge(&mut self.text_bytes, bytes, MAX_PDF_TOTAL_TEXT,
            "PDF cumulative text budget")
    }

    fn elements(&mut self, count: usize) -> anyhow::Result<()> {
        Self::charge(&mut self.emitted_elements, count, MAX_PDF_EMITTED_ELEMENTS,
            "PDF emitted element budget")
    }

    fn operations(&mut self, count: usize) -> anyhow::Result<()> {
        Self::charge(&mut self.operations, count, MAX_PDF_OPERATIONS,
            "PDF cumulative operation budget")
    }
}

/// Bounded checksum/EOF validation for ordinary single-filter zlib PDF
/// streams. lopdf 0.45 logs zlib errors and can return truncated/empty data
/// as Ok(Vec), which violates the Complete-or-error indexing boundary.
fn validate_flate_integrity(raw: &[u8], max_output: usize) -> anyhow::Result<()> {
    let mut decoder = flate2::bufread::ZlibDecoder::new(raw);
    let mut sink = Vec::new();
    let cap = (max_output as u64).saturating_add(1);
    (&mut decoder).take(cap).read_to_end(&mut sink)?;
    anyhow::ensure!(
        sink.len() <= max_output,
        "PDF FlateDecode output budget exceeded"
    );
    anyhow::ensure!(
        decoder.get_ref().is_empty(),
        "PDF FlateDecode has trailing bytes after compressed stream"
    );
    Ok(())
}

/// lopdf's convenience method silently skips dangling /Contents references
/// and may use compressed bytes as fallback when decoding fails. A security
/// extractor must never turn those cases into an empty or corrupted index.
fn checked_page_content(pdf: &PdfDocument, page_id: ObjectId) -> anyhow::Result<Vec<u8>> {
    let page_dict = pdf.get_dictionary(page_id)?;
    let content_ids = pdf.get_page_contents(page_id);
    match page_dict.get(b"Contents") {
        Err(_) => return Ok(Vec::new()), // A genuinely blank page is valid.
        Ok(Object::Array(items)) => {
            anyhow::ensure!(
                items.iter().all(|item| matches!(item, Object::Reference(_)))
                    && items.len() == content_ids.len(),
                "PDF /Contents array contains unsupported or missing references"
            );
        }
        Ok(Object::Reference(_)) => {
            anyhow::ensure!(
                !content_ids.is_empty(),
                "PDF /Contents reference cannot be resolved"
            );
        }
        Ok(_) => anyhow::bail!("PDF /Contents must be an indirect stream or an array"),
    }

    let mut page_content = Vec::new();
    for id in content_ids {
        let stream = pdf.get_object(id)?.as_stream()?;
        // A malformed /Filter value is not permission to parse raw bytes.
        let filters = if stream.dict.get(b"Filter").is_ok() {
            stream.filters()?
        } else {
            Vec::new()
        };
        let remaining = MAX_DECOMPRESSED_STREAM
            .checked_sub(page_content.len())
            .ok_or_else(|| anyhow::anyhow!("PDF decoded page byte budget exceeded"))?;
        anyhow::ensure!(remaining > 0, "PDF decoded page byte budget exceeded");
        // Reserve one byte for the newline separator between page streams.
        // lopdf can return Ok(empty) after logging a Flate checksum error;
        // explicitly require a valid zlib EOF before using its output.
        if filters.len() == 1 && filters[0] == b"FlateDecode" {
            validate_flate_integrity(&stream.content, remaining - 1)?;
        }
        let decoded = stream.get_plain_content_with_limit(remaining - 1)?;
        anyhow::ensure!(
            decoded.len() < remaining,
            "PDF decoded page byte budget exceeded"
        );
        page_content.extend_from_slice(&decoded);
        page_content.push(b'\n');
    }
    Ok(page_content)
}

fn parse_page(
    page_id: ObjectId,
    pdf_document: &PdfDocument,
    elements: &mut Vec<Element>,
    budget: &mut PdfBudget,
) -> anyhow::Result<()> {
    fn collect_text(
        text: &mut String,
        encoding: &Encoding<'_>,
        operands: &[Object],
        elements: &mut Vec<Element>,
        budget: &mut PdfBudget,
        depth: usize,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            depth <= MAX_PDF_TEXT_ARRAY_DEPTH,
            "PDF text array nesting budget exceeded"
        );
        for operand in operands {
            // Never log the raw operand: it may contain private document text.
            match operand {
                Object::String(bytes, _) => {
                    let decoded_text = PdfDocument::decode_text(encoding, bytes)?;
                    anyhow::ensure!(
                        !decoded_text.contains('\u{FFFD}'),
                        "PDF glyph could not be mapped to Unicode"
                    );
                    budget.text(decoded_text.len())?;
                    text.push_str(&decoded_text);

                    // Preserve Shiva's original special list marker handling
                    // without cloning the entire existing paragraph/list.
                    if bytes.len() == 1 && bytes[0] == 1 {
                        if elements.is_empty() {
                            budget.elements(1)?;
                            elements.push(List { elements: vec![], numbered: false });
                        } else if matches!(elements.last(), Some(List { .. })) {
                            budget.elements(2)?;
                            if let Some(List { elements: list_items, .. }) = elements.last_mut() {
                                list_items.push(ListItem {
                                    element: Text {
                                        text: std::mem::take(text),
                                        size: 8,
                                    },
                                });
                            }
                        } else if matches!(elements.last(), Some(Paragraph { .. })) {
                            budget.elements(3)?;
                            if let Some(Paragraph { elements: paragraph_items }) = elements.last_mut() {
                                paragraph_items.push(Text {
                                    text: std::mem::take(text),
                                    size: 8,
                                });
                            }
                            elements.push(List { elements: vec![], numbered: false });
                        } else {
                            budget.elements(1)?;
                            elements.push(List { elements: vec![], numbered: false });
                        }
                    }
                }
                Object::Array(arr) => {
                    collect_text(text, encoding, arr, elements, budget, depth + 1)?;
                    budget.text(1)?;
                    text.push(' ');
                }
                Object::Integer(i) if *i < -100 => {
                    budget.text(1)?;
                    text.push(' ');
                }
                _ => {}
            }
        }
        Ok(())
    }

    let mut text = String::new();
    let fonts = pdf_document.get_page_fonts(page_id)?;
    anyhow::ensure!(fonts.len() <= 64, "PDF per-page font count budget exceeded");
    let mut encodings = BTreeMap::new();
    for (name, font) in fonts {
        let composite = font.get(b"Subtype")
            .and_then(Object::as_name)
            .map(|name| name == b"Type0")
            .unwrap_or(false);
        let encoding = font.get_font_encoding_with_limit(
            pdf_document, MAX_TOUNICODE_CMAP,
        )?;
        anyhow::ensure!(
            !composite || matches!(&encoding, Encoding::UnicodeMapEncoding(_)),
            "composite PDF font has no usable ToUnicode mapping"
        );
        encodings.insert(name, encoding);
    }

    let page = checked_page_content(pdf_document, page_id)?;
    budget.page_content(page.len())?;
    let content = Content::decode(&page)?;
    budget.operations(content.operations.len())?;

    let mut current_encoding = None;
    for operation in &content.operations {
        match operation.operator.as_ref() {
            "Tm" => {
                budget.elements(if elements.is_empty() { 2 } else { 1 })?;
                let item = Text { text: std::mem::take(&mut text), size: 8 };
                if elements.is_empty() {
                    elements.push(Paragraph { elements: vec![item] });
                } else if let Some(Paragraph { elements: paragraph_items }) = elements.last_mut() {
                    paragraph_items.push(item);
                } else {
                    elements.push(item);
                }
            }
            "Tf" => {
                let font = operation.operands.first()
                    .ok_or(ParserError::Common)?.as_name()?;
                current_encoding = Some(encodings.get(font)
                    .ok_or_else(|| anyhow::anyhow!("PDF text references an unknown font"))?);
            }
            "Tj" | "TJ" => {
                let encoding = current_encoding
                    .ok_or_else(|| anyhow::anyhow!("PDF text without a usable font"))?;
                collect_text(&mut text, encoding, &operation.operands, elements, budget, 0)?;
            }
            "ET" => {
                if !text.ends_with('\n') {
                    budget.text(1)?;
                    text.push('\n');
                }
            }
            _ => {}
        }
    }

    if !text.is_empty() {
        let item = Text { text, size: 8 };
        if elements.is_empty() {
            budget.elements(2)?;
            elements.push(Paragraph { elements: vec![item] });
        } else if let Some(Paragraph { elements: paragraph_items }) = elements.last_mut() {
            budget.elements(1)?;
            paragraph_items.push(item);
        } else if let Some(List { elements: list_items, .. }) = elements.last_mut() {
            budget.elements(2)?;
            list_items.push(ListItem { element: item });
        } else {
            // The previous parser silently discarded trailing text here.
            budget.elements(1)?;
            elements.push(item);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::*;
    use crate::pdf::{
        PdfBudget, Transformer, MAX_DECOMPRESSED_STREAM, MAX_PDF_EMITTED_ELEMENTS,
        MAX_PDF_TOTAL_PAGE_CONTENT, MAX_PDF_TOTAL_TEXT,
    };
    use crate::{markdown, pdf};
    use lopdf::LoadOptions;
    use bytes::Bytes;
    use log::{debug, info};
    use std::collections::HashMap;

    #[test]
    fn test() -> anyhow::Result<()> {
        let pdf = std::fs::read("test/data/document.pdf")?;
        let pdf_bytes = Bytes::from(pdf);
        let parsed = Transformer::parse(&pdf_bytes);
        assert!(parsed.is_ok());
        let parsed_document = parsed.unwrap();
        info!("==========================");
        info!("{:?}", parsed_document);
        info!("==========================");
        let generated_result = Transformer::generate(&parsed_document)?;
        std::fs::write("test/data/generated.pdf", generated_result)?;
        Ok(())
    }

    #[test]
    fn test_md() -> anyhow::Result<()> {
        let document = std::fs::read("test/data/document.md")?;
        let documents_bytes = Bytes::from(document);
        let parsed_document = markdown::Transformer::parse_with_loader(
            &documents_bytes,
            disk_image_loader("test/data"),
        )?;
        debug!("==========================");
        debug!("{:?}", parsed_document);
        debug!("==========================");
        let generated_result = Transformer::generate(&parsed_document)?;
        std::fs::write("test/data/generated.pdf", generated_result)?;
        Ok(())
    }

    #[test]
    fn test_list() -> anyhow::Result<()> {
        let document = std::fs::read("test/data/document.md")?;
        let documents_bytes = Bytes::from(document);
        let mut images = HashMap::new();
        let image_bytes = std::fs::read("test/data/picture.png")?;
        let image_bytes = Bytes::from(image_bytes);
        images.insert("image0.png".to_string(), image_bytes);
        let parsed = markdown::Transformer::parse_with_loader(
            &documents_bytes,
            disk_image_loader("test/data"),
        );
        assert!(parsed.is_ok());
        let mut parsed_document = parsed.unwrap();
        debug!("==========================");
        debug!("{:?}", parsed_document);
        debug!("==========================");
        parsed_document.set_page_header(vec![Element::Text {
            text: "header".to_string(),
            size: 10,
        }]);

        parsed_document.set_page_footer(vec![Element::Text {
            text: "footer".to_string(),
            size: 10,
        }]);
        let generated_result = Transformer::generate(&parsed_document);
        assert!(generated_result.is_ok());
        std::fs::write("test/data/typst.pdf", generated_result.unwrap())?;

        Ok(())
    }


    /// Regression: a PDF page with two /Contents streams is ONE logical
    /// page. Each stream must appear exactly once in extracted text.
    /// Previously the outer loop called get_page_content() once per stream,
    /// duplicating both stream texts.
    #[test]
    fn multi_stream_page_does_not_duplicate_extracted_text() -> anyhow::Result<()> {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document as LopdfDocument, Object, Stream};

        fn text_stream(value: &str) -> anyhow::Result<Stream> {
            let operations = vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal(value)]),
                Operation::new("ET", vec![]),
            ];
            Ok(Stream::new(dictionary! {}, Content { operations }.encode()?))
        }

        let mut pdf = LopdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let font_id = pdf.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = pdf.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let first_id = pdf.add_object(text_stream("AlphaUnique")?);
        let second_id = pdf.add_object(text_stream("BetaUnique")?);
        let page_id = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => vec![first_id.into(), second_id.into()],
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }));
        let catalog_id = pdf.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        pdf.trailer.set("Root", catalog_id);
        assert_eq!(pdf.get_page_contents(page_id).len(), 2);

        let mut output = Vec::new();
        pdf.save_to(&mut output)?;
        let parsed = Transformer::parse(&Bytes::from(output))?;
        // Debug only for test assertions: no input text is logged by worker.
        let flattened = format!("{parsed:?}");
        assert_eq!(flattened.matches("AlphaUnique").count(), 1, "{flattened}");
        assert_eq!(flattened.matches("BetaUnique").count(), 1, "{flattened}");
        Ok(())
    }


    // A minimal Type0/Identity-H page containing explicit UTF-16BE CMap
    // mappings. These are character codes, NOT UTF-8 glyph bytes.
    // This fixture exercises both beginbfchar and beginbfrange.
    fn cid_identity_h_fixture(with_cmap: bool, code_bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document as LopdfDocument, Object, Stream, StringFormat};

        const CMAP: &str = r#"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
2 beginbfchar
<0001> <041F>
<0002> <0440>
endbfchar
1 beginbfrange
<0003> <0004> <4E16>
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end"#;

        let mut pdf = LopdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let cmap_id = pdf.add_object(Stream::new(dictionary! {}, CMAP.as_bytes().to_vec()));
        let mut font = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "FixtureCIDFont",
            "Encoding" => "Identity-H",
        };
        if with_cmap {
            font.set("ToUnicode", Object::Reference(cmap_id));
        }
        let font_id = pdf.add_object(font);
        let resources_id = pdf.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });

        // U+041F (П), U+0440 (р), U+4E16 (世), U+4E17 (丗).
        // Identity-H means TWO bytes per glyph code.
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Tj", vec![Object::String(
                    code_bytes.to_vec(),
                    StringFormat::Hexadecimal,
                )]),
                Operation::new("ET", vec![]),
            ],
        };
        let stream_id = pdf.add_object(Stream::new(dictionary! {}, content.encode()?));
        let page_id = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => resources_id,
            "Contents" => stream_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }));
        let catalog_id = pdf.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        pdf.trailer.set("Root", catalog_id);
        let mut output = Vec::new();
        pdf.save_to(&mut output)?;
        Ok(output)
    }

    #[test]
    fn cid_identity_h_tounicode_preserves_unicode() -> anyhow::Result<()> {
        let bytes = cid_identity_h_fixture(true, &[0, 1, 0, 2, 0, 3, 0, 4])?;
        let parsed = Transformer::parse(&Bytes::from(bytes))?;
        let output = format!("{parsed:?}");
        assert!(output.contains("Пр世丗"), "ToUnicode output lost Cyrillic/CJK: {output}");
        assert_eq!(output.matches("Пр世丗").count(), 1, "duplicated text: {output}");
        assert!(!output.contains('\u{FFFD}'), "Unicode replacement leaked into parsed text");
        Ok(())
    }

    #[test]
    fn cid_identity_h_without_cmap_fails_closed() -> anyhow::Result<()> {
        let bytes = cid_identity_h_fixture(false, &[0, 1, 0, 2, 0, 3, 0, 4])?;
        let result = Transformer::parse(&Bytes::from(bytes));
        assert!(result.is_err(), "CID text without ToUnicode must never look Complete");
        Ok(())
    }

    #[test]
    fn cid_identity_h_unmapped_glyph_fails_closed() -> anyhow::Result<()> {
        let bytes = cid_identity_h_fixture(true, &[0, 1, 0, 9])?;
        let err = Transformer::parse(&Bytes::from(bytes))
            .expect_err("unmapped CID must not be silently indexed");
        assert!(
            err.to_string().contains("glyph could not be mapped"),
            "unexpected missing-CID error: {err}"
        );
        Ok(())
    }

    /// The combined uncompressed page budget applies even if the PDF bytes
    /// contain a tiny Flate-compressed stream. The negative must fail because
    /// of the decompression ceiling, not because the PDF was malformed.
    #[test]
    fn cid_page_decompression_budget_fails_closed() -> anyhow::Result<()> {
        use lopdf::{dictionary, Document as LopdfDocument, Object, Stream};

        let mut pdf = LopdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let mut stream = Stream::new(
            dictionary! {},
            vec![b' '; MAX_DECOMPRESSED_STREAM + 512],
        );
        stream.compress()?;
        let stream_id = pdf.add_object(stream);
        let page_id = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => dictionary! {},
            "Contents" => stream_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }));
        let catalog_id = pdf.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        pdf.trailer.set("Root", catalog_id);
        let mut output = Vec::new();
        pdf.save_to(&mut output)?;
        assert!(
            output.len() < MAX_DECOMPRESSED_STREAM / 2,
            "fixture should remain small on disk and inflate past the limit"
        );

        let loaded = LopdfDocument::load_mem_with_options(
            &output,
            LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED_STREAM),
        )?;
        let err = loaded
            .get_page_content_with_limit(page_id, MAX_DECOMPRESSED_STREAM)
            .expect_err("page must exceed combined uncompressed budget");
        assert!(
            format!("{err:?}").contains("MemoryLimitExceeded"),
            "unexpected rejection: {err:?}"
        );
        assert!(
            Transformer::parse(&Bytes::from(output)).is_err(),
            "Shiva must reject an oversized decoded PDF page"
        );
        Ok(())
    }


    #[test]
    fn total_budget_counters_reject_overflow_without_wraparound() -> anyhow::Result<()> {
        let mut budget = PdfBudget::default();
        budget.text(MAX_PDF_TOTAL_TEXT)?;
        assert!(budget.text(1).is_err(), "text ceiling must be enforced");
        let mut budget = PdfBudget::default();
        budget.page_content(MAX_PDF_TOTAL_PAGE_CONTENT)?;
        assert!(budget.page_content(1).is_err(), "global page ceiling must be enforced");
        let mut budget = PdfBudget::default();
        budget.elements(MAX_PDF_EMITTED_ELEMENTS)?;
        assert!(budget.elements(1).is_err(), "element count must be enforced");
        let mut budget = PdfBudget::default();
        assert!(budget.text(usize::MAX).is_err(), "counter wraparound must fail");
        Ok(())
    }

    #[test]
    fn total_budget_rejects_five_individually_valid_compressed_pages() -> anyhow::Result<()> {
        use lopdf::{dictionary, Document as LopdfDocument, Object, Stream};

        const PAGE_DECODED_BYTES: usize = 4 * 1024 * 1024 + 64;
        assert!(PAGE_DECODED_BYTES < MAX_DECOMPRESSED_STREAM);
        assert!(5 * PAGE_DECODED_BYTES > MAX_PDF_TOTAL_PAGE_CONTENT);

        let mut pdf = LopdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let mut page_ids = Vec::new();
        for _ in 0..5 {
            let mut page_stream = Stream::new(
                dictionary! {}, vec![b' '; PAGE_DECODED_BYTES],
            );
            page_stream.compress()?;
            let stream_id = pdf.add_object(page_stream);
            let page_id = pdf.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => stream_id,
                "Resources" => dictionary! {},
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            });
            page_ids.push(page_id);
        }
        pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64,
        }));
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        pdf.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes)?;
        assert!(bytes.len() < 1_000_000, "compressed test fixture unexpectedly large");
        let err = Transformer::parse(&Bytes::from(bytes))
            .expect_err("individually legal pages must exceed combined decode budget");
        assert!(
            err.to_string().contains("cumulative decoded page budget"),
            "aggregate budget error expected: {err}"
        );
        Ok(())
    }

    #[test]
    fn total_budget_rejects_unicode_output_across_two_pages() -> anyhow::Result<()> {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document as LopdfDocument, Object, Stream, StringFormat};

        const CHARS_PER_PAGE: usize = 5 * 1024 * 1024;
        assert!(CHARS_PER_PAGE * 2 > MAX_PDF_TOTAL_TEXT);
        let mut pdf = LopdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let font_id = pdf.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id = pdf.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let mut page_ids = Vec::new();
        for _ in 0..2 {
            let ops = Content { operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Tj", vec![Object::String(
                    vec![b'A'; CHARS_PER_PAGE], StringFormat::Literal,
                )]),
                Operation::new("ET", vec![]),
            ] };
            let mut stream = Stream::new(dictionary! {}, ops.encode()?);
            stream.compress()?;
            let contents = pdf.add_object(stream);
            let page_id = pdf.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "Contents" => contents, "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            });
            page_ids.push(page_id);
        }
        pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => 2,
        }));
        let catalog = pdf.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id,
        });
        pdf.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes)?;
        assert!(bytes.len() < 1_000_000, "fixture must be highly compressed");
        let err = Transformer::parse(&Bytes::from(bytes))
            .expect_err("two individually acceptable pages exceed global output budget");
        assert!(err.to_string().contains("PDF cumulative text budget"),
            "output text must be rejected by the global limit: {err}");
        Ok(())
    }

    /// Repeated text matrix operators must append in source order without
    /// cloning all preceding paragraph children at every step.
    #[test]
    fn total_budget_many_tm_operations_keep_order() -> anyhow::Result<()> {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document as LopdfDocument, Object, Stream};

        let mut pdf = LopdfDocument::with_version("1.5");
        let pages_id = pdf.new_object_id();
        let font = pdf.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources = pdf.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font },
        });
        let mut ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
        ];
        for i in 0..1500 {
            ops.push(Operation::new(
                "Tj", vec![Object::string_literal(format!("TM_{i:04}_MARKER"))],
            ));
            ops.push(Operation::new("Tm", vec![]));
        }
        ops.push(Operation::new("ET", vec![]));
        let content = Content { operations: ops };
        let stream_id = pdf.add_object(Stream::new(dictionary! {}, content.encode()?));
        let page = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => resources,
            "Contents" => stream_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
        }));
        let catalog = pdf.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id,
        });
        pdf.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes)?;
        let result = Transformer::parse(&Bytes::from(bytes))?;
        let debug_output = format!("{result:?}");
        assert_eq!(debug_output.matches("_MARKER").count(), 1500);
        assert!(debug_output.find("TM_0000_MARKER").unwrap()
            < debug_output.find("TM_1499_MARKER").unwrap());
        Ok(())
    }

    #[test]
    fn test_minimal_pdf_offline() -> anyhow::Result<()> {
        let document = Document::new(vec![Element::Text {
            text: "Shiva offline PDF smoke".to_owned(),
            size: 10,
        }]);
        let pdf = Transformer::generate(&document)?;
        assert!(pdf.starts_with(b"%PDF-"), "generated bytes are not a PDF");
        Ok(())
    }

    #[test]
    fn test_hyperlink_generation() -> anyhow::Result<()> {
        use Element::*;
        let elements = vec![
            Paragraph {
                elements: vec![
                    Text {
                        text: "Line 1".to_owned(),
                        size: 8,
                    },
                    Text {
                        text: "Line 2".to_owned(),
                        size: 8,
                    },
                    Text {
                        text: "Line 3".to_owned(),
                        size: 8,
                    },
                ],
            },
            Hyperlink {
                title: "Example".to_owned(),
                url: "https://www.example.com".to_owned(),
                alt: "Example Site".to_owned(),
                size: 8,
            },
            Hyperlink {
                title: "GitHub".to_owned(),
                url: "https://www.github.com".to_owned(),
                alt: "GitHub".to_owned(),
                size: 8,
            },
        ];
        let document = Document::new(elements);

        debug!("==========================");
        debug!("{:?}", document);
        debug!("==========================");

        let generated_result = Transformer::generate(&document)?;

        std::fs::write(
            "test/data/generated_hyperlink.pdf",
            generated_result,
        )?;

        Ok(())
    }

    #[test]
    fn simple_test() {
        let content = std::fs::read("test/data/test.txt").unwrap();
        let md_content = String::from_utf8(content).unwrap();
        let input_bytes = Bytes::from(md_content);
        let document = markdown::Transformer::parse(&input_bytes).unwrap();
        let output_bytes = pdf::Transformer::generate(&document).unwrap().to_vec();

        std::fs::write("test/data/test.pdf", output_bytes).unwrap();
    }
}

// Security QA uses deterministic malformed PDFs kept out of release code.
#[cfg(test)]
mod hostile_tests;
