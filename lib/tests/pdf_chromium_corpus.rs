//! QA-only text completeness against a third, independent PDF writer: Chromium.
//! Synthetic HTML-print fixtures do not enable PDF in shipping workers.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;
use std::path::{Path, PathBuf};

const MAX_QA_SIZE: usize = 2 * 1024 * 1024;

fn fixture_dir() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var_os("SHIVA_QA_CHROMIUM_DIR")
            .expect("QA Chromium fixture path must be explicitly supplied"),
    );
    assert!(dir.is_absolute() && dir.is_dir(), "QA Chromium fixture directory missing");
    dir
}

fn read_profile(root: &Path, name: &str) -> Bytes {
    assert!(
        matches!(
            name,
            "chromium-1-page.pdf" | "chromium-8-pages.pdf" | "chromium-unicode-2-pages.pdf"
        ),
        "unexpected QA profile filename"
    );
    let path = root.join(name);
    assert!(!path.is_symlink(), "QA input must not be a symlink");
    let raw = std::fs::read(path).expect("Chrome synthetic QA PDF missing");
    assert!(
        (100..=MAX_QA_SIZE).contains(&raw.len()) && raw.starts_with(b"%PDF-"),
        "Chrome PDF failed header/size gate"
    );
    Bytes::from(raw)
}

fn verify_chromium_pdf(root: &Path, name: &str, pages: usize, unicode: bool) {
    let input = read_profile(root, name);
    let pdf = lopdf::Document::load_mem(&input).expect("Chrome PDF must be well formed");
    assert_eq!(
        pdf.get_pages().len(), pages,
        "Chrome writer printed a different number of pages: {name}"
    );
    let info_id = pdf.trailer.get(b"Info")
        .expect("Chrome PDF Info missing")
        .as_reference()
        .expect("Chrome PDF Info is not an indirect object");
    let producer = pdf.get_dictionary(info_id)
        .expect("Chrome Info dictionary missing")
        .get(b"Producer")
        .expect("Chrome PDF Producer missing")
        .as_str()
        .expect("Chrome PDF Producer is not a text string");
    assert!(
        producer.starts_with(b"Skia/PDF"),
        "wrong independent PDF producer for Chrome source"
    );

    let parsed = Transformer::parse(&input)
        .expect("Chrome printed valid PDF but Shiva did not extract complete text");
    let output = format!("{parsed:?}");
    let mut previous = None;
    for page in 1..=pages {
        let checks = [
            format!("CHQA-{page:03}-OF-{pages:03}"),
            format!("CHBODY-{page:03}-OF-{pages:03}-ALPHA-12345"),
            format!("CHTAIL-{page:03}-OF-{pages:03}-OMEGA-67890"),
        ];
        for marker in &checks {
            assert_eq!(
                output.matches(marker).count(), 1,
                "missing or duplicate Chrome page text marker: {name} / {marker}"
            );
            let at = output.find(marker).expect("validated page text marker");
            if let Some(before) = previous {
                assert!(at > before, "Chrome PDF text or pages out of source order: {name}");
            }
            previous = Some(at);
        }
    }
    if unicode {
        assert_eq!(
            output.matches("Тестовая строка кириллицы").count(),
            pages,
            "Chrome Cyrillic text disappeared or duplicated"
        );
    } else {
        assert_eq!(
            output.matches("INDEPENDENT-CHROME-PDF-WRITER").count(),
            pages,
            "Chrome non-Unicode body text disappeared or duplicated"
        );
    }
    println!("SHIVA_CHROMIUM_TEXT_ORDER=pass;name={name};pages={pages};unicode={unicode}");
}

#[test]
fn independent_chromium_writer_preserves_page_count_text_order_and_cyrillic() {
    let root = fixture_dir();
    verify_chromium_pdf(&root, "chromium-1-page.pdf", 1, false);
    verify_chromium_pdf(&root, "chromium-8-pages.pdf", 8, false);
    verify_chromium_pdf(&root, "chromium-unicode-2-pages.pdf", 2, true);
    println!("SHIVA_CHROMIUM_ORACLE=pass;profiles=3;pages=11;text_markers=33");
}
