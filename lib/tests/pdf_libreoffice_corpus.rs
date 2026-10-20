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
            let offset = output[cursor..].find(line).unwrap_or_else(|| {
                panic!(
                    "synthetic QA PDF text missing or reordered: page={page}, expected={line:?}, \
                     occurrences={}, first_position={:?}, cursor={}, extracted_bytes={}, prefix={:?}",
                    output.matches(line).count(), output.find(line), cursor, output.len(),
                    output.chars().take(240).collect::<String>()
                )
            });
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


/// 0x01 is a legitimate font code. Its meaning comes from /ToUnicode;
/// LibreOffice uses 0x01 for 'L'. Treating it as a list token fragments
/// later lines even when the PDF's whole source text is intact.
#[test]
fn pdf_code_0x01_is_a_valid_unicode_glyph_not_a_list_item() {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document as LopdfDocument, Object, Stream, StringFormat};
    use shiva::core::Element;

    const CMAP: &[u8] = b"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<00> <FF>
endcodespacerange
2 beginbfchar
<01> <004C>
<02> <004F>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end";

    let mut pdf = LopdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let cmap_id = pdf.add_object(Stream::new(dictionary! {}, CMAP.to_vec()));
    let font_id = pdf.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "ToUnicode" => cmap_id,
    });
    let resources_id = pdf.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let glyph_line = || {
        Operation::new("TJ", vec![Object::Array(vec![
            Object::String(vec![1], StringFormat::Hexadecimal),
            Object::String(vec![2], StringFormat::Hexadecimal),
        ])])
    };
    let operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
        glyph_line(),
        Operation::new("ET", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
        glyph_line(),
        Operation::new("ET", vec![]),
    ];
    let stream_id = pdf.add_object(Stream::new(
        dictionary! {},
        Content { operations }.encode().expect("test PDF content encoding"),
    ));
    let page_id = pdf.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => stream_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    }));
    let catalog_id = pdf.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    pdf.trailer.set("Root", catalog_id);
    let mut raw = Vec::new();
    pdf.save_to(&mut raw).expect("save deterministic PDF fixture");
    let document = Transformer::parse(&Bytes::from(raw))
        .expect("valid ToUnicode source PDF must extract");
    let mut flattened = String::new();
    for band in &document.bands {
        for element in band.elements() {
            match element {
                Element::Text { text, .. } => flattened.push_str(text),
                Element::Paragraph { elements } => {
                    for sub in elements {
                        match sub {
                            Element::Text { text, .. } => flattened.push_str(text),
                            _ => panic!("unexpected nested PDF text element"),
                        }
                    }
                }
                Element::List { .. } => panic!("glyph byte 0x01 became a phantom list"),
                _ => panic!("unexpected PDF output element"),
            }
        }
    }
    assert_eq!(
        flattened.matches("LO").count(), 2,
        "both repeated source lines must be emitted contiguously"
    );
    println!("SHIVA_PDF_0X01_CMAP=pass;decoded_lines=2");
}
