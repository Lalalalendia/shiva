//! QA-only qpdf structural rewrites; no shipping PDF worker changes.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;
use std::path::{Path, PathBuf};

const MAX_PDF_BYTES: usize = 4 * 1024 * 1024;
const SOURCES: [(&str, &str, usize); 7] = [
    ("cc0-1", "cc0", 1),
    ("cc0-10", "cc0", 10),
    ("rl-1", "reportlab", 1),
    ("rl-12", "reportlab", 12),
    ("ch-1", "chromium", 1),
    ("ch-8", "chromium", 8),
    ("ch-uni-2", "chromium", 2),
];
const REWRITES: [&str; 3] = ["linearized", "objstreams", "uncompressed"];

fn corpus() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("SHIVA_QA_QPDF_DIR")
            .expect("verified qpdf QA output path must be supplied"),
    );
    assert!(root.is_absolute() && root.is_dir() && !root.is_symlink());
    root
}

fn read_pdf(root: &Path, label: &str, variant: &str) -> Bytes {
    assert!(SOURCES.iter().any(|(name, _, _)| *name == label));
    assert!(variant == "original" || REWRITES.contains(&variant));
    let path = root.join(label).join(format!("{variant}.pdf"));
    assert!(!path.is_symlink() && path.is_file(), "QA source absent/symlinked");
    let raw = std::fs::read(path).expect("verified QA PDF not readable");
    assert!(
        (100..=MAX_PDF_BYTES).contains(&raw.len()) && raw.starts_with(b"%PDF-"),
        "QA PDF exceeded size/header bounds"
    );
    Bytes::from(raw)
}

fn validate_source_markers(text: &str, origin: &str, n: usize) {
    let mut prev = None;
    for page in 1..=n {
        let marker = match origin {
            "cc0" => format!("Page {page} of {n}"),
            "reportlab" => format!("RLQA-{page:03}-OF-{n:03}"),
            "chromium" => format!("CHQA-{page:03}-OF-{n:03}"),
            _ => panic!("invalid independent PDF producer"),
        };
        assert_eq!(
            text.matches(&marker).count(), 1,
            "source marker lost or duplicated: producer={origin} page={page}"
        );
        let position = text.find(&marker).expect("validated marker missing");
        if let Some(previous) = prev {
            assert!(position > previous, "original page text out of order");
        }
        prev = Some(position);
    }
}

fn entire_text(bytes: &Bytes, expected_pages: usize) -> String {
    let pdf = lopdf::Document::load_mem(bytes).expect("malformed qpdf output");
    assert_eq!(pdf.get_pages().len(), expected_pages, "page count changed");
    let document = Transformer::parse(bytes).expect("qpdf PDF extraction rejected");
    format!("{document:?}")
}

#[test]
fn qpdf_rewrites_preserve_complete_extraction_and_order() {
    let root = corpus();
    let mut rewritten_pages = 0;
    for (label, producer, pages) in SOURCES {
        let original = read_pdf(&root, label, "original");
        let baseline = entire_text(&original, pages);
        validate_source_markers(&baseline, producer, pages);
        if label == "ch-uni-2" {
            assert_eq!(
                baseline.matches("Тестовая строка кириллицы").count(),
                2,
                "Cyrillic source not complete"
            );
        }
        for variant in REWRITES {
            let rewritten = read_pdf(&root, label, variant);
            assert_ne!(
                original.as_ref(), rewritten.as_ref(), "not a real byte rewrite"
            );
            let extracted = entire_text(&rewritten, pages);
            assert!(
                extracted == baseline,
                "qpdf rewrite changed whole Shiva extraction: source={label} variant={variant}"
            );
            rewritten_pages += pages;
            println!(
                "SHIVA_QPDF_FULL_TEXT=pass;source={label};variant={variant};pages={pages}"
            );
        }
    }
    assert_eq!(rewritten_pages, 105, "unexpected transformed page count");
    println!("SHIVA_QPDF_METAMORPHIC_QA=pass;originals=7;rewrites=21;rewritten_pages=105");
}
