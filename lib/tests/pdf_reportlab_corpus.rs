//! Independent ReportLab-generated PDF QA, not a shipping file-reader feature.
//! This intentionally does not use the same PDF writer as Shiva/lopdf or the
//! pdf-lib-generated CC0 corpus. Failures are fidelity bugs, not "known skips".
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;
use std::path::{Path, PathBuf};

const MAX_QA_SIZE: usize = 2 * 1024 * 1024;

fn fixture_dir() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("SHIVA_QA_REPORTLAB_DIR")
            .expect("QA ReportLab fixture path must be supplied explicitly"),
    );
    assert!(root.is_absolute() && root.is_dir(), "missing ephemeral QA directory");
    root
}

fn read_profile(root: &Path, name: &str) -> Bytes {
    assert!(
        matches!(
            name,
            "reportlab-1-page.pdf"
                | "reportlab-12-pages.pdf"
                | "reportlab-rotated-3-pages.pdf"
        ),
        "unknown ReportLab fixture filename"
    );
    let input = std::fs::read(root.join(name)).expect("CI-generated reportlab fixture missing");
    assert!(
        (100..=MAX_QA_SIZE).contains(&input.len()) && input.starts_with(b"%PDF-"),
        "ReportLab PDF size/header gate failed"
    );
    Bytes::from(input)
}

fn check_independent_page_markers(root: &Path, name: &str, count: usize) {
    let raw = read_profile(root, name);
    let parsed = Transformer::parse(&raw)
        .expect("independently generated PDF must not silently lose an extractable page");
    let result = format!("{parsed:?}");
    let mut last_start = None;
    for n in 1..=count {
        let marker = format!("RLQA-{n:03}-OF-{count:03}");
        assert_eq!(
            result.matches(&marker).count(), 1,
            "ReportLab page marker missing or duplicated: {name}, page={n}"
        );
        let at = result.find(&marker).expect("marker must be present");
        if let Some(before) = last_start {
            assert!(at > before, "ReportLab page order changed: {name}, page={n}");
        }
        last_start = Some(at);
    }
    println!("SHIVA_QA_RL_TEXT_ORDER=pass;name={name};pages={count}");
}

#[test]
fn reportlab_independent_pdf_generator_preserves_complete_text_and_page_order() {
    let root = fixture_dir();
    check_independent_page_markers(&root, "reportlab-1-page.pdf", 1);
    check_independent_page_markers(&root, "reportlab-12-pages.pdf", 12);
    check_independent_page_markers(&root, "reportlab-rotated-3-pages.pdf", 3);

    // Structural fixture check: the 3-page file actually uses rotations.
    // This is an independent input sanity assertion, *not* the completeness
    // oracle used above. Verify the generated graph is not a mislabeled copy.
    let raw = read_profile(&root, "reportlab-rotated-3-pages.pdf");
    let pdf = lopdf::Document::load_mem(&raw).expect("valid ReportLab PDF");
    let pages = pdf.get_pages();
    assert_eq!(pages.len(), 3, "rotated fixture must have exactly 3 pages");
    for (page, expected_rotation) in [(1, 0i64), (2, 90), (3, 180)] {
        let id = pages[&page];
        let actual = pdf.get_dictionary(id)
            .expect("page must have a dictionary")
            .get(b"Rotate")
            .expect("ReportLab must emit per-page rotation")
            .as_i64()
            .expect("rotation value must be integer");
        assert_eq!(actual, expected_rotation, "fixture PDF rotation drift at page {page}");
    }
    println!("SHIVA_QA_RL_ROTATION=pass;degrees=0,90,180");
    println!("SHIVA_QA_RL_COMPLETE=pass;profiles=3;text_pages=16");
}
