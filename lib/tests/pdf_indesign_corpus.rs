//! Independent real-world Adobe InDesign PDF QA against source-derived text.
//!
//! This is not PDF admission for Akula or a shipping worker. The only input is
//! the previously verified fixed Git blob + exact SHA-256 in ephemeral CI.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::{Document, Element, TransformerTrait};
use shiva::pdf::Transformer;
use std::path::PathBuf;

const INPUT_NAME: &str = "indesign-neddy-flyer.pdf";
const INPUT_SIZE: usize = 59_106;

// Ground truth decoded from the PDF's embedded /ToUnicode tables and ordered
// original content streams (InDesign CS 3.0 / Adobe PDF Library 6.0). The
// Poppler side of the CI independently inspects the fixed identical input.
// Do not weaken these markers to make a lossy extraction test green.
const REQUIRED_TEXT_IN_SOURCE_ORDER: [&str; 8] = [
    "The Denver",
    "Puppet Guild",
    "Presents:",
    "Neddy Buys",
    "a New Pillow",
    "Monday, July 10th",
    "SML Coffee",
    "The Dream Job!",
];

fn flattened_document(document: &Document) -> String {
    fn append_text(element: &Element, output: &mut String) {
        match element {
            Element::Text { text, .. } => output.push_str(text),
            Element::Paragraph { elements } => {
                for child in elements {
                    append_text(child, output);
                }
            }
            Element::List { .. } => panic!("PDF glyph codes must not create a phantom List"),
            _ => panic!("unexpected element from PDF text-only parser"),
        }
    }

    let mut output = String::new();
    for band in &document.bands {
        for element in band.elements() {
            append_text(element, &mut output);
        }
    }
    output
}

#[test]
fn genuine_indesign_pdf_retains_complete_source_phrases_in_order() {
    let root = PathBuf::from(
        std::env::var_os("SHIVA_QA_INDESIGN_DIR")
            .expect("CI must pass the exact pinned external PDF directory"),
    );
    assert!(root.is_absolute() && root.is_dir() && !root.is_symlink());
    let path = root.join(INPUT_NAME);
    assert!(path.is_file() && !path.is_symlink(), "verified CC0 input absent");
    let data = std::fs::read(path).expect("cannot read pinned InDesign PDF");
    assert_eq!(data.len(), INPUT_SIZE, "PDF source size changed");
    assert!(data.starts_with(b"%PDF-"), "PDF header changed");

    let input = Bytes::from(data);
    let pdf = lopdf::Document::load_mem(&input)
        .expect("PDF source cannot pass the independent page-tree parser");
    assert_eq!(pdf.get_pages().len(), 1, "InDesign source has exactly one page");

    // Success is not allowed to mean an empty document or missing text:
    // every complete source phrase must appear once and in page-content order.
    let parsed = Transformer::parse(&input)
        .expect("Shiva must not silently produce an incomplete real PDF result");
    let text = flattened_document(&parsed);
    assert!(!text.contains('\u{fffd}'), "unmapped glyph replacement is not Complete");
    let mut previous_end = 0;
    for phrase in REQUIRED_TEXT_IN_SOURCE_ORDER {
        assert_eq!(
            text.matches(phrase).count(),
            1,
            "required real InDesign phrase absent or duplicated: {phrase:?}"
        );
        let pos = text.find(phrase).expect("verified phrase must be present");
        assert!(
            pos >= previous_end,
            "real InDesign text order differs from the source: {phrase:?}"
        );
        previous_end = pos + phrase.len();
        println!("SHIVA_IND_FIDELITY_MARKER=pass;phrase={phrase:?}");
    }
    println!(
        "SHIVA_IND_SEMANTIC_ORACLE=pass;pages=1;source_phrases=8;lost=0;duplicated=0"
    );
}
