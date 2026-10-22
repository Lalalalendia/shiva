//! SHIVA-PDF-08: bounded, opt-in PDF source fidelity diagnostics.
//!
//! The old parse() path remains compatible but has no verified Complete status.
//! This test uses deterministic independent PDFs, not private user files.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::{Band, Document, Element, TransformerTrait};
use shiva::pdf::{PdfFidelityIssue, PdfFidelityStatus, Transformer};
use std::path::PathBuf;

fn root(variable: &str) -> PathBuf {
    let value = std::env::var_os(variable).expect("QA source corpus missing");
    let root = PathBuf::from(value);
    assert!(root.is_absolute() && root.is_dir());
    root
}

fn extract(root_var: &str, fixture: &str) -> anyhow::Result<shiva::pdf::PdfExtractionReport> {
    assert!(matches!(
        fixture,
        "healthy.pdf" | "broken.pdf" | "mixed.pdf" | "image_only.pdf"
            | "invisible.pdf" | "form_xobject.pdf"
    ));
    let bytes = std::fs::read(root(root_var).join(fixture))?;
    assert!(bytes.starts_with(b"%PDF-") && bytes.len() < 512 * 1024);
    Transformer::parse_with_diagnostics(&Bytes::from(bytes))
}

fn text(doc: &Document) -> String {
    fn walk(elements: &[Element], out: &mut String) {
        for item in elements {
            match item {
                Element::Text { text, .. } => out.push_str(text),
                Element::Paragraph { elements } => walk(elements, out),
                _ => {}
            }
        }
    }
    let mut out = String::new();
    for band in &doc.bands {
        if let Band::Detail(elements) = band {
            walk(elements, &mut out);
        }
    }
    out
}

const VISUAL: &str = "SHIVA_QA_VISUAL_TRUTH_DIR";
const OPERATORS: &str = "SHIVA_QA_TEXT_OPERATORS_DIR";

#[test]
fn normal_native_text_is_never_silently_complete() -> anyhow::Result<()> {
    let report = extract(VISUAL, "healthy.pdf")?;
    assert_eq!(report.status, PdfFidelityStatus::NeedsReview);
    assert_eq!(report.pages.len(), 1);
    assert_eq!(report.pages[0].page_number, 1);
    assert_eq!(report.pages[0].issues, vec![PdfFidelityIssue::VisualFidelityUnverified]);
    assert_eq!(text(&report.document).trim(), "HELLO SHIVA");
    Ok(())
}

#[test]
fn valid_but_wrong_tounicode_stays_unverified() -> anyhow::Result<()> {
    let report = extract(VISUAL, "broken.pdf")?;
    assert_eq!(report.status, PdfFidelityStatus::NeedsReview);
    assert_eq!(text(&report.document).trim(), "XELLO ZXIVA");
    // Nothing in this metadata pretends the map can be verified by syntax.
    assert_eq!(report.pages[0].issues, vec![PdfFidelityIssue::VisualFidelityUnverified]);
    Ok(())
}

#[test]
fn skipped_raster_content_is_partial_even_with_native_text() -> anyhow::Result<()> {
    for name in ["image_only.pdf", "mixed.pdf"] {
        let report = extract(VISUAL, name)?;
        assert_eq!(report.status, PdfFidelityStatus::Partial);
        assert!(report.pages[0].issues.contains(&PdfFidelityIssue::UnprocessedXObject));
    }
    Ok(())
}

#[test]
fn bounded_form_text_remains_unverified_not_complete() -> anyhow::Result<()> {
    let report = extract(OPERATORS, "form_xobject.pdf")?;
    assert_eq!(report.status, PdfFidelityStatus::NeedsReview);
    assert_eq!(report.pages.len(), 1);
    assert_eq!(report.pages[0].issues, vec![PdfFidelityIssue::VisualFidelityUnverified]);
    assert!(text(&report.document).contains("PAGE_NATIVE_123"));
    assert!(text(&report.document).contains("FORM_INSIDE_451"));
    Ok(())
}

#[test]
fn invisible_render_mode_is_flagged_without_dropping_ocr_layer() -> anyhow::Result<()> {
    let report = extract(VISUAL, "invisible.pdf")?;
    assert_eq!(report.status, PdfFidelityStatus::NeedsReview);
    assert!(report.pages[0].issues.contains(&PdfFidelityIssue::InvisibleTextRenderModeObserved));
    assert!(text(&report.document).contains("HIDDEN MEMO"));
    Ok(())
}

#[test]
fn legacy_parse_output_is_not_modified_by_diagnostic_collection() -> anyhow::Result<()> {
    let bytes = std::fs::read(root(VISUAL).join("mixed.pdf"))?;
    let plain = Transformer::parse(&Bytes::from(bytes.clone()))?;
    let report = Transformer::parse_with_diagnostics(&Bytes::from(bytes))?;
    assert_eq!(text(&plain), text(&report.document));
    Ok(())
}
