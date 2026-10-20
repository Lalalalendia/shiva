//! Opt-in external PDF library QA. Files are pinned CC0 fixture bytes fetched
//! by the separate CI verification script into an ephemeral directory.
//! Never read arbitrary repository, user, or installed-Akula file paths.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::{Document, TransformerTrait};
use shiva::pdf::Transformer;
use std::path::{Path, PathBuf};

const MAX_SAMPLE_BYTES: usize = 2 * 1024 * 1024;

fn input_dir() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("SHIVA_QA_PINNED_PDF_DIR")
            .expect("pinned corpus CI must provide its verified directory"),
    );
    assert!(path.is_absolute() && path.is_dir(), "missing verified PDF QA directory");
    path
}

fn read_sample(root: &Path, basename: &str) -> Bytes {
    assert!(
        matches!(
            basename,
            "sample-1-page.pdf"
                | "sample-10-pages.pdf"
                | "sample-100-pages.pdf"
                | "sample-blank-a4.pdf"
                | "sample-scanned.pdf"
                | "sample-corrupted.pdf"
                | "sample-password-protected.pdf"
        ),
        "QA fixture not allowlisted"
    );
    let raw = std::fs::read(root.join(basename))
        .expect("exact SHA-pinned CC0 sample must exist");
    assert!(
        raw.len() < MAX_SAMPLE_BYTES && raw.starts_with(b"%PDF-"),
        "QA input exceeds bounds or lacks PDF header"
    );
    Bytes::from(raw)
}

fn verify_exact_pages(root: &Path, filename: &str, n: usize) {
    let sample = read_sample(root, filename);
    let parsed = Transformer::parse(&sample)
        .expect("valid third-party PDF must not silently lose its text pages");
    let extracted = format!("{parsed:?}");

    let mut previous = None;
    for page in 1..=n {
        let marker = format!("Page {page} of {n}");
        let hits = extracted.matches(&marker).count();
        assert_eq!(
            hits, 1, "third-party PDF page marker absent/duplicated: page={page} of {n}"
        );
        let position = extracted.find(&marker)
            .expect("all validated PDF page markers must be emitted");
        if let Some(before) = previous {
            assert!(
                position > before,
                "third-party PDF pages changed their original /Kids ordering"
            );
        }
        previous = Some(position);
    }
    println!("SHIVA_QA_CC0_TEXT_PAGES=pass;pages={n}");
}

#[test]
fn pinned_external_pdf_corpus_has_explicit_completeness_oracles() {
    let root = input_dir();

    // This is an independent-generator fidelity gate, not just "no crash".
    verify_exact_pages(&root, "sample-1-page.pdf", 1);
    verify_exact_pages(&root, "sample-10-pages.pdf", 10);
    verify_exact_pages(&root, "sample-100-pages.pdf", 100);

    let blank = Transformer::parse(&read_sample(&root, "sample-blank-a4.pdf"))
        .expect("valid blank A4 PDF must parse");
    let empty_document = format!("{:?}", Document::new(Vec::new()));
    assert_eq!(
        format!("{blank:?}"),
        empty_document,
        "blank A4 PDF unexpectedly contains searchable text"
    );
    println!("SHIVA_QA_CC0_BLANK_TEXT=empty");

    // A scan may be rejected or parsed into an empty document; neither
    // constitutes OCR. Explicitly classify it as Needs OCR, never Complete.
    if let Ok(scan) = Transformer::parse(&read_sample(&root, "sample-scanned.pdf")) {
        assert_eq!(
            format!("{scan:?}"),
            empty_document,
            "image-only PDF must not invent a complete searchable text layer"
        );
    }
    println!("SHIVA_QA_CC0_SCAN=needs_ocr");

    assert!(
        Transformer::parse(&read_sample(&root, "sample-corrupted.pdf")).is_err(),
        "truncated external PDF must not be accepted as Complete"
    );
    println!("SHIVA_QA_CC0_CORRUPT=reject");

    assert!(
        Transformer::parse(&read_sample(&root, "sample-password-protected.pdf")).is_err(),
        "encrypted external PDF must not be indexed without authorization"
    );
    println!("SHIVA_QA_CC0_ENCRYPTED=reject");
    println!("SHIVA_QA_CC0_ORACLE_GATE=pass;fixtures=7");
}
