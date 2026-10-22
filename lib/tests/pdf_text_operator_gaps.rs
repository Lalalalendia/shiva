//! SHIVA-PDF-06: characterize text-operator and Form XObject fidelity gaps.
//!
//! The ordinary text operators must now preserve all six native strings.
//! Form XObject text is still a documented incomplete extraction and must
//! never be used as an unconditional Complete/worker admission proof.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::{Band, Document, Element, TransformerTrait};
use shiva::pdf::Transformer;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var_os("SHIVA_QA_TEXT_OPERATORS_DIR")
            .expect("missing independent text-operator QA directory"),
    );
    assert!(dir.is_absolute() && dir.is_dir(), "invalid QA directory");
    dir
}

fn read_pdf(name: &str) -> Vec<u8> {
    assert!(
        matches!(name, "text_operators.pdf" | "form_xobject.pdf"),
        "unexpected fixture filename"
    );
    let bytes = std::fs::read(fixtures().join(name))
        .expect("expected generated PDF fixture");
    assert!(bytes.starts_with(b"%PDF-"));
    assert!((100..=32768).contains(&bytes.len()), "unbounded fixture size");
    bytes
}

fn append_text(items: &[Element], out: &mut String) {
    for item in items {
        match item {
            Element::Text { text, .. } => out.push_str(text),
            Element::Paragraph { elements } => append_text(elements, out),
            _ => {}
        }
    }
}

fn extract(raw: Vec<u8>) -> anyhow::Result<String> {
    let doc: Document = Transformer::parse(&Bytes::from(raw))?;
    let mut text = String::new();
    for band in doc.bands {
        if let Band::Detail(items) = band {
            append_text(&items, &mut text);
        }
    }
    Ok(text.trim().to_owned())
}

#[test]
fn native_pdf_text_operators_preserve_all_six_painted_lines() -> anyhow::Result<()> {
    let text = extract(read_pdf("text_operators.pdf"))?;
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "ALPHA_FIRST",
            "BETA_SECOND",
            "GAMMA_THIRD",
            "DELTA_QUOTE",
            "EPSILON_DOUBLE",
            "ZETA_TD",
        ],
        "Shiva must preserve all six native painted text lines in order"
    );
    println!("SHIVA_PDF_OPERATOR_REPAIR=pass;painted_text=6/6;order=true");
    Ok(())
}

#[test]
fn native_pdf_form_xobject_text_is_omitted() -> anyhow::Result<()> {
    let text = extract(read_pdf("form_xobject.pdf"))?;
    assert_eq!(text.matches("PAGE_NATIVE_123").count(), 1);
    assert!(!text.contains("FORM_INSIDE_451"),
        "Form XObject unexpectedly parsed (update expectations)");
    println!("SHIVA_PDF_KNOWN_GAP=form_xobject;visible_nested_text_missing=true");
    Ok(())
}
