//! Bounded native Form XObject traversal, including cycles and repetition.
//! QA fixtures are synthetic PDFs, not attacker-provided network downloads.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::{Band, Document, Element};
use shiva::pdf::{PdfFidelityIssue, PdfFidelityStatus, Transformer};
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    assert!(matches!(name,
        "form_nested.pdf" | "form_cycle.pdf" | "form_repeat.pdf" |
        "form_deep.pdf" | "form_font_fanout.pdf"));
    let root = PathBuf::from(
        std::env::var_os("SHIVA_QA_TEXT_OPERATORS_DIR")
            .expect("generated PDF QA directory is missing"),
    );
    assert!(root.is_absolute() && root.is_dir());
    let raw = std::fs::read(root.join(name)).expect("generated Form PDF is missing");
    assert!(raw.starts_with(b"%PDF-") && raw.len() < 32768);
    raw
}

fn text(doc: &Document) -> String {
    fn walk(items: &[Element], out: &mut String) {
        for item in items {
            match item {
                Element::Text { text, .. } => out.push_str(text),
                Element::Paragraph { elements } => walk(elements, out),
                _ => {}
            }
        }
    }
    let mut out = String::new();
    for band in &doc.bands {
        if let Band::Detail(items) = band {
            walk(items, &mut out);
        }
    }
    out
}

#[test]
fn nested_form_text_follows_paint_invocation_order() -> anyhow::Result<()> {
    let result = Transformer::parse_with_diagnostics(&Bytes::from(fixture("form_nested.pdf")))?;
    let actual = text(&result.document);
    assert_eq!(result.status, PdfFidelityStatus::NeedsReview);
    for name in ["PAGE_NESTED", "OUTER_NESTED", "INNER_NESTED"] {
        assert_eq!(actual.matches(name).count(), 1);
    }
    assert!(actual.find("PAGE_NESTED") < actual.find("OUTER_NESTED"));
    assert!(actual.find("OUTER_NESTED") < actual.find("INNER_NESTED"));
    println!("SHIVA_PDF_FORM_NESTED=pass;levels=2;native_text_markers=3");
    Ok(())
}

#[test]
fn form_reference_cycle_stops_with_partial_provenance() -> anyhow::Result<()> {
    let result = Transformer::parse_with_diagnostics(&Bytes::from(fixture("form_cycle.pdf")))?;
    assert_eq!(result.status, PdfFidelityStatus::Partial);
    assert!(result.pages[0].issues.contains(&PdfFidelityIssue::UnprocessedXObject));
    assert_eq!(text(&result.document).matches("CYCLE_SAFE").count(), 1);
    Ok(())
}

#[test]
fn repeat_invocations_obey_cumulative_page_budget() -> anyhow::Result<()> {
    let result = Transformer::parse_with_diagnostics(&Bytes::from(fixture("form_repeat.pdf")))?;
    assert_eq!(result.status, PdfFidelityStatus::Partial);
    assert!(result.pages[0].issues.contains(&PdfFidelityIssue::UnprocessedXObject));
    assert_eq!(text(&result.document).matches("REPEAT_SAFE").count(), 128);
    Ok(())
}

#[test]
fn nested_form_depth_ceiling_prevents_unbounded_recursion() -> anyhow::Result<()> {
    let result = Transformer::parse_with_diagnostics(&Bytes::from(fixture("form_deep.pdf")))?;
    assert_eq!(result.status, PdfFidelityStatus::Partial);
    assert!(result.pages[0].issues.contains(&PdfFidelityIssue::UnprocessedXObject));
    let actual = text(&result.document);
    for depth in 0..4 {
        assert_eq!(actual.matches(&format!("DEPTH_{depth}")).count(), 1);
    }
    for depth in 4..6 {
        assert!(!actual.contains(&format!("DEPTH_{depth}")));
    }
    Ok(())
}

#[test]
fn repeated_form_font_decoding_has_a_separate_page_budget() -> anyhow::Result<()> {
    let result = Transformer::parse_with_diagnostics(&Bytes::from(fixture("form_font_fanout.pdf")))?;
    assert_eq!(result.status, PdfFidelityStatus::Partial);
    assert!(result.pages[0].issues.contains(&PdfFidelityIssue::UnprocessedXObject));
    assert_eq!(text(&result.document).matches("DOUBLE_FONTS").count(), 64);
    Ok(())
}
