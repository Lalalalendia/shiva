use crate::core::Element::{List, Paragraph, Text};
use crate::core::{Document, Element, ListItem, ParserError, TransformerTrait};

use anyhow;
use bytes::Bytes;
use log::{debug, warn};
use lopdf::content::Content;
use lopdf::encodings::Encoding;
use lopdf::{Document as PdfDocument, LoadOptions, Object, ObjectId};
use std::collections::BTreeMap;
use typst::{eval::Tracer, foundations::Smart};

pub struct Transformer;

// Standalone library QA limits, not permission to enable PDF in the worker.
// These bounds cover the input file, eager load-time stream decoding,
// individual fonts' CMaps and each page's combined uncompressed content.
// Process CPU, total allocations and document output budgets still need a
// separate sandbox gate before any Akula integration.
const MAX_PDF_INPUT: usize = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_STREAM: usize = 8 * 1024 * 1024;
const MAX_TOUNICODE_CMAP: usize = 1024 * 1024;
const MAX_PDF_PAGES: usize = 1024;

impl TransformerTrait for Transformer {
    fn parse(document: &Bytes) -> anyhow::Result<Document> {
        let mut elements: Vec<Element> = Vec::new();
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
            parse_page(page_id, &pdf_document, &mut elements)?;
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

fn parse_page(
    page_id: ObjectId,
    pdf_document: &PdfDocument,
    elements: &mut Vec<Element>,
) -> anyhow::Result<()> {
    fn collect_text(
        text: &mut String,
        encoding: &Encoding<'_>,
        operands: &[Object],
        elements: &mut Vec<Element>,
    ) -> anyhow::Result<()> {
        for operand in operands.iter() {
            debug!("2 {:?}", operand);
            match *operand {
                Object::String(ref bytes, _) => {
                    let decoded_text = PdfDocument::decode_text(encoding, bytes)?;
                    // A missing CID code would otherwise become U+FFFD and
                    // risk publishing corrupted tokens as "Complete" text.
                    anyhow::ensure!(
                        !decoded_text.contains('\u{FFFD}'),
                        "PDF glyph could not be mapped to Unicode"
                    );
                    text.push_str(&decoded_text);
                    if bytes.len() == 1 && bytes[0] == 1 {
                        match elements.last() {
                            None => {
                                let list_element = List {
                                    elements: vec![],
                                    numbered: false,
                                };
                                elements.push(list_element);
                            }
                            Some(el) => {
                                match el {
                                    List { .. } => {
                                        let old_list = elements.pop().unwrap();
                                        // let list = old_list.list_as_ref()?;
                                        if let List {
                                            elements: list_elements,
                                            numbered,
                                        } = old_list
                                        {
                                            let mut list_item_elements = list_elements.clone();
                                            let text_element = Text {
                                                text: text.clone(),
                                                size: 8,
                                            };
                                            let new_list_item_element = ListItem {
                                                element: text_element,
                                            };
                                            list_item_elements.push(new_list_item_element);
                                            let new_list = List {
                                                elements: list_item_elements,
                                                numbered,
                                            };
                                            elements.push(new_list);
                                            text.clear();
                                        }
                                    }
                                    Paragraph { .. } => {
                                        let old_paragraph = elements.pop().unwrap();
                                        // let paragraph = old_paragraph.paragraph_as_ref()?;
                                        if let Paragraph {
                                            elements: paragraph_elements,
                                        } = old_paragraph
                                        {
                                            let mut paragraph_elements = paragraph_elements.clone();
                                            let text_element = Text {
                                                text: text.clone(),
                                                size: 8,
                                            };
                                            paragraph_elements.push(text_element);
                                            let new_paragraph = Paragraph {
                                                elements: paragraph_elements,
                                            };
                                            elements.push(new_paragraph);
                                            text.clear();

                                            let list_element = List {
                                                elements: vec![],
                                                numbered: false,
                                            };
                                            elements.push(list_element);
                                        }
                                    }
                                    _ => {
                                        let list_element = List {
                                            elements: vec![],
                                            numbered: false,
                                        };
                                        elements.push(*Box::new(list_element));
                                    }
                                }
                            }
                        }
                    }
                }
                Object::Array(ref arr) => {
                    collect_text(text, encoding, arr, elements)?;
                    text.push(' ');
                }
                Object::Integer(i) => {
                    if i < -100 {
                        text.push(' ');
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let mut text = String::new();

    let fonts = pdf_document.get_page_fonts(page_id)?;
    let mut encodings = BTreeMap::new();
    for (name, font) in fonts {
        // Don't silently fall back to a Latin single-byte encoding for
        // composite CID fonts. They require a real ToUnicode CMap.
        let composite = font.get(b"Subtype")
            .and_then(Object::as_name)
            .map(|name| name == b"Type0")
            .unwrap_or(false);
        let encoding = font.get_font_encoding_with_limit(
            pdf_document, MAX_TOUNICODE_CMAP,
        )?;
        anyhow::ensure!(
            !composite || matches!(encoding, Encoding::UnicodeMapEncoding(_)),
            "composite PDF font has no usable ToUnicode mapping"
        );
        encodings.insert(name, encoding);
    }

    // The bounded variant is essential: page /Contents can include many
    // individually compressed streams, each contributing to decompression.
    let vec = pdf_document.get_page_content_with_limit(
        page_id, MAX_DECOMPRESSED_STREAM,
    )?;
    let content = Content::decode(&vec)?;
    let mut current_encoding = None;
    for operation in &content.operations {
        debug!("1 {:?}", operation.operator);
        match operation.operator.as_ref() {
            "Tm" => {
                let text_element = Text {
                    text: text.clone(),
                    size: 8,
                };
                match elements.last() {
                    None => {
                        let paragraph_element = Paragraph {
                            elements: vec![text_element],
                        };
                        elements.push(paragraph_element);
                    }
                    Some(el) => match el {
                        Paragraph { .. } => {
                            let old_paragraph = elements.pop().unwrap();
                            if let Paragraph {
                                elements: paragraph_elements,
                            } = old_paragraph
                            {
                                let mut paragraph_elements = paragraph_elements.clone();
                                paragraph_elements.push(text_element);
                                let new_paragraph = Paragraph {
                                    elements: paragraph_elements,
                                };
                                elements.push(new_paragraph);
                            }
                        }
                        _ => {
                            elements.push(text_element);
                        }
                    },
                }
                text.clear();
            }
            "Tf" => {
                let current_font = operation
                    .operands
                    .first()
                    .ok_or(ParserError::Common)?
                    .as_name()?;
                current_encoding = Some(
                    encodings.get(current_font)
                        .ok_or_else(|| anyhow::anyhow!("PDF text references an unknown font"))?
                );
            }
            "Tj" | "TJ" => {
                let encoding = current_encoding
                    .ok_or_else(|| anyhow::anyhow!("PDF text without a usable font"))?;
                collect_text(&mut text, encoding, &operation.operands, elements)?;
            }
            "ET" => {
                if !text.ends_with('\n') {
                    text.push('\n')
                }
            }
            _ => {}
        }
    }

    if !text.is_empty() {
        let text_element = Text {
            text: text.clone(),
            size: 8,
        };
        match elements.last() {
            None => {
                let paragraph_element = Paragraph {
                    elements: vec![text_element],
                };
                elements.push(*Box::new(paragraph_element));
            }
            Some(el) => {
                match el {
                    Paragraph { .. } => {
                        let old_paragraph = elements.pop().unwrap();
                        if let Paragraph {
                            elements: paragraph_elements,
                        } = old_paragraph
                        {
                            let mut paragraph_elements = paragraph_elements.clone();
                            paragraph_elements.push(text_element);
                            let new_paragraph = Paragraph {
                                elements: paragraph_elements,
                            };
                            elements.push(*Box::new(new_paragraph));
                        }
                    }
                    List { .. } => {
                        let old_list = elements.pop().unwrap();
                        // let list = old_list.list_as_ref()?;
                        if let List {
                            elements: list_elements,
                            numbered,
                        } = old_list
                        {
                            let mut list_item_elements = list_elements.clone();
                            let new_list_item_element = ListItem {
                                element: text_element,
                            };
                            list_item_elements.push(new_list_item_element);
                            let new_list = List {
                                elements: list_item_elements,
                                numbered,
                            };
                            elements.push(*Box::new(new_list));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::*;
    use crate::pdf::Transformer;
    use crate::{markdown, pdf};
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
    fn cid_identity_h_fixture(with_cmap: bool) -> anyhow::Result<Vec<u8>> {
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
                    vec![0, 1, 0, 2, 0, 3, 0, 4],
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
        let bytes = cid_identity_h_fixture(true)?;
        let parsed = Transformer::parse(&Bytes::from(bytes))?;
        let output = format!("{parsed:?}");
        assert!(output.contains("Пр世丗"), "ToUnicode output lost Cyrillic/CJK: {output}");
        assert_eq!(output.matches("Пр世丗").count(), 1, "duplicated text: {output}");
        assert!(!output.contains('\u{FFFD}'), "Unicode replacement leaked into parsed text");
        Ok(())
    }

    #[test]
    fn cid_identity_h_without_cmap_fails_closed() -> anyhow::Result<()> {
        let bytes = cid_identity_h_fixture(false)?;
        let result = Transformer::parse(&Bytes::from(bytes));
        assert!(result.is_err(), "CID text without ToUnicode must never look Complete");
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
