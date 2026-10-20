//! LibreOffice Writer PDF QA, using independent synthetic DOCX source text.
//! Library-only feature: no PDF/DOCX admission in installed Shiva worker.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;
use std::path::{Path, PathBuf};

const MAX_PDF_BYTES: usize = 2 * 1024 * 1024;
const PROFILES: [(&str, usize, bool); 3] = [
    ("writer-1-page.pdf", 1, false),
    ("writer-6-pages.pdf", 6, false),
    ("writer-unicode-2-pages.pdf", 2, true),
];

fn corpus_dir() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var_os("SHIVA_QA_LIBREOFFICE_DIR")
            .expect("LibreOffice fixture directory must be explicitly supplied"),
    );
    assert!(dir.is_absolute() && dir.is_dir() && !dir.is_symlink());
    dir
}

fn read_profile(root: &Path, filename: &str) -> Bytes {
    assert!(PROFILES.iter().any(|(name, _, _)| *name == filename));
    let path = root.join(filename);
    assert!(path.is_file() && !path.is_symlink(), "QA fixture missing/symlinked");
    let raw = std::fs::read(path).expect("Writer-exported PDF unreadable");
    assert!(
        (100..=MAX_PDF_BYTES).contains(&raw.len()) && raw.starts_with(b"%PDF-"),
        "LibreOffice QA PDF size/header violation"
    );
    Bytes::from(raw)
}

fn assert_source_text(output: &str, pages: usize, unicode: bool) {
    let mut cursor = 0;
    for page in 1..=pages {
        let lines = [
            format!("LOQA-{page:03}-OF-{pages:03}"),
            format!("LOBODY-{page:03}-OF-{pages:03}-ALPHA-12345"),
            format!("LOTAIL-{page:03}-OF-{pages:03}-OMEGA-67890"),
            if unicode {
                "Тестовая строка LibreOffice".to_string()
            } else {
                "WRITER-PDF-FROM-SYNTHETIC-DOCX".to_string()
            },
        ];
        for line in &lines {
            let offset = output[cursor..].find(line)
                .expect("synthetic DOCX source text missing or out of page order");
            cursor += offset + line.len();
        }
        for line in &lines[..3] {
            assert_eq!(output.matches(line).count(), 1, "source marker duplicated");
        }
    }
    let repeated = if unicode {
        "Тестовая строка LibreOffice"
    } else {
        "WRITER-PDF-FROM-SYNTHETIC-DOCX"
    };
    assert_eq!(
        output.matches(repeated).count(),
        pages,
        "source text duplicate or lost across Writer pages"
    );
}

#[test]
fn libreoffice_writer_export_preserves_full_source_lines_page_order_and_unicode() {
    let root = corpus_dir();
    let mut checked_pages = 0;
    for (filename, pages, unicode) in PROFILES {
        let input = read_profile(&root, filename);
        let pdf = lopdf::Document::load_mem(&input)
            .expect("Writer exported invalid PDF");
        assert_eq!(pdf.get_pages().len(), pages, "writer output PDF page count drift");
        let doc = Transformer::parse(&input)
            .expect("Shiva failed to parse a valid LibreOffice PDF");
        let extraction = format!("{doc:?}");
        assert_source_text(&extraction, pages, unicode);
        checked_pages += pages;
        println!(
            "SHIVA_LO_TEXT_ORDER=pass;profile={filename};pages={pages};unicode={unicode}"
        );
    }
    assert_eq!(checked_pages, 9);
    println!("SHIVA_LO_FIDELITY=pass;profiles=3;pages=9;source_lines=36");
}
