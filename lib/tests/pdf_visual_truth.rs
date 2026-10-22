//! P0 QA: actual Shiva PDF parser's visual-vs-Unicode fidelity gaps.
//!
//! These are known-problem characterization tests, not correctness proof and
//! not a production-grade OCR or font repair gate. A visual-byte-equivalent
//! PDF with corrupt ToUnicode is currently accepted as ordinary native text.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::{Band, Document, Element, TransformerTrait};
use shiva::pdf::Transformer;
use std::path::PathBuf;

fn corpus() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var_os("SHIVA_QA_VISUAL_TRUTH_DIR")
            .expect("missing QA-only visual truth directory"),
    );
    assert!(dir.is_absolute() && dir.is_dir(), "invalid QA-only fixture root");
    dir
}

fn load(name: &str) -> Vec<u8> {
    assert!(
        matches!(name, "healthy" | "broken" | "image_only" | "mixed" | "invisible"),
        "unapproved PDF profile"
    );
    let raw = std::fs::read(corpus().join(format!("{name}.pdf")))
        .expect("CI-generated PDF fixture missing");
    assert!(raw.starts_with(b"%PDF-") && (100..=512 * 1024).contains(&raw.len()));
    raw
}

fn append_text(elements: &[Element], out: &mut String) {
    for item in elements {
        match item {
            Element::Text { text, .. } => out.push_str(text),
            Element::Paragraph { elements } => append_text(elements, out),
            _ => {}
        }
    }
}

fn shiva_text(raw: Vec<u8>) -> anyhow::Result<String> {
    let document: Document = Transformer::parse(&Bytes::from(raw))?;
    let mut out = String::new();
    for band in &document.bands {
        if let Band::Detail(elements) = band {
            append_text(elements, &mut out);
        }
    }
    Ok(out.trim().to_owned())
}

#[test]
fn valid_pdf_is_indexed_exactly_once() -> anyhow::Result<()> {
    let actual = shiva_text(load("healthy"))?;
    assert_eq!(actual, "HELLO SHIVA", "healthy PDF control changed");
    println!("SHIVA_PDF_VISUAL_CONTROL=pass;expected=HELLO_SHIVA");
    Ok(())
}

#[test]
fn syntactically_valid_wrong_tounicode_is_not_visual_truth() -> anyhow::Result<()> {
    let good = load("healthy");
    let bad = load("broken");
    // Both PDFs have unchanged page content/Helvetica outlines and xref.
    // Only two CMap destinations change: H -> X and S -> Z.
    assert_eq!(good.len(), bad.len());
    assert_eq!(
        good.iter().zip(&bad).filter(|(left, right)| left != right).count(),
        2,
        "mutation must alter ONLY two Unicode mappings"
    );
    let actual_good = shiva_text(good)?;
    let actual_bad = shiva_text(bad)?;
    assert_eq!(actual_good, "HELLO SHIVA");
    assert_eq!(actual_bad, "XELLO ZXIVA");
    assert_ne!(actual_good, actual_bad);
    println!(
        "SHIVA_PDF_KNOWN_GAP=wrong_tounicode;parser_accepts_wrong_unicode=true;visual_same=true"
    );
    // These diagnostic observations intentionally DO NOT approve Shiva's
    // output as Complete. Update this test with the future refusal/review gate.
    Ok(())
}

#[test]
fn image_only_pdf_has_no_native_text_layer() -> anyhow::Result<()> {
    let actual = shiva_text(load("image_only"))?;
    assert!(actual.is_empty(), "raster glyphs cannot be ordinary PDF text");
    println!("SHIVA_PDF_KNOWN_GAP=image_only;native_text=empty;ocr_needed=true");
    Ok(())
}

#[test]
fn mixed_pdf_native_extraction_misses_raster_label() -> anyhow::Result<()> {
    let actual = shiva_text(load("mixed"))?;
    assert_eq!(actual, "NATIVE HEADER");
    assert!(!actual.contains("IMAGE LABEL"));
    println!("SHIVA_PDF_KNOWN_GAP=mixed;missing_visible_image_label=true");
    Ok(())
}

#[test]
fn invisible_pdf_text_is_still_emitted_by_native_parser() -> anyhow::Result<()> {
    let bytes = load("invisible");
    assert!(bytes.windows(b"3 Tr".len()).any(|part| part == b"3 Tr"));
    let actual = shiva_text(bytes)?;
    assert!(actual.contains("PUBLIC MESSAGE"));
    assert!(actual.contains("HIDDEN MEMO"));
    // Invisible text may be a legitimate scan OCR layer; the safe policy
    // needs source attribution, not an unconditional removal rule.
    println!("SHIVA_PDF_KNOWN_GAP=invisible_text_indexed=true;policy_unresolved=true");
    Ok(())
}
