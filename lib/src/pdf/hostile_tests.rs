//! Reproducible, synthetic PDF invalid-input corpus. No network/files,
//! secrets, downloaded samples or intentionally executable PDF payloads.
//! These cases test the *library* fail-closed boundary, not worker admission.
use super::{Transformer, MAX_PDF_TEXT_ARRAY_DEPTH, MAX_TOUNICODE_CMAP};
use crate::core::TransformerTrait;
use bytes::Bytes;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as LopdfDocument, Object, Stream, StringFormat};

enum FixtureFont {
    Type1,
    Type0Cmap(Vec<u8>),
    NoFont,
}

const BASE_CMAP: &str = r#"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 beginbfchar
<0001> <041F>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end"#;

/// Build a structurally complete PDF. By generating the input with lopdf,
/// negative tests cannot accidentally pass only because the PDF lacks a
/// valid catalog, page tree, or xref. Missing references are deliberate.
fn fixture(
    font: FixtureFont,
    operations: Vec<Operation>,
    dangling_contents: bool,
) -> anyhow::Result<Vec<u8>> {
    let mut pdf = LopdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let resources_id = match font {
        FixtureFont::NoFont => pdf.add_object(dictionary! {}),
        FixtureFont::Type1 => {
            let font_id = pdf.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
            });
            pdf.add_object(dictionary! {
                "Font" => dictionary! { "F1" => font_id },
            })
        }
        FixtureFont::Type0Cmap(cmap) => {
            let mut stream = Stream::new(dictionary! {}, cmap);
            stream.compress()?;
            let cmap_id = pdf.add_object(stream);
            let font_id = pdf.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type0",
                "BaseFont" => "FixtureCIDFont",
                "Encoding" => "Identity-H",
                "ToUnicode" => cmap_id,
            });
            pdf.add_object(dictionary! {
                "Font" => dictionary! { "F1" => font_id },
            })
        }
    };
    let content = Content { operations };
    let content_id = pdf.add_object(Stream::new(dictionary! {}, content.encode()?));
    let actual_contents = if dangling_contents {
        Object::Reference(pdf.new_object_id())
    } else {
        Object::Reference(content_id)
    };
    let page_id = pdf.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => actual_contents,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
    }));
    let catalog_id = pdf.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id,
    });
    pdf.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    pdf.save_to(&mut out)?;
    Ok(out)
}

fn latin_ops(operands: Vec<Object>) -> Vec<Operation> {
    vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
        Operation::new("Tj", operands),
        Operation::new("ET", vec![]),
    ]
}

fn cid_ops() -> Vec<Operation> {
    latin_ops(vec![Object::String(
        vec![0, 1], StringFormat::Hexadecimal,
    )])
}

#[test]
fn hostile_control_valid_pdf_remains_indexable() -> anyhow::Result<()> {
    let raw = fixture(
        FixtureFont::Type1,
        latin_ops(vec![Object::string_literal("SafeBaselineMarker")]),
        false,
    )?;
    let parsed = Transformer::parse(&Bytes::from(raw))?;
    let rendered = format!("{parsed:?}");
    assert_eq!(rendered.matches("SafeBaselineMarker").count(), 1);
    Ok(())
}

#[test]
fn hostile_control_empty_filter_list_preserves_valid_text() -> anyhow::Result<()> {
    // An empty /Filter array is a valid uncompressed stream declaration.
    // Rejecting corrupt streams must not erase the text of valid PDFs.
    let bytes = fixture(
        FixtureFont::Type1,
        latin_ops(vec![Object::string_literal("EmptyFilterStillText")]),
        false,
    )?;
    let mut pdf = LopdfDocument::load_mem(&bytes)?;
    let page_id = *pdf.get_pages().values().next().unwrap();
    let contents_id = *pdf.get_page_contents(page_id).first().unwrap();
    let stream = pdf.get_object_mut(contents_id)?.as_stream_mut()?;
    stream.dict.set("Filter", Object::Array(vec![]));
    let mut reserialized = Vec::new();
    pdf.save_to(&mut reserialized)?;
    let parsed = Transformer::parse(&Bytes::from(reserialized))?;
    assert_eq!(format!("{parsed:?}").matches("EmptyFilterStillText").count(), 1);
    Ok(())
}

#[test]
fn hostile_cmap_invalid_bfrange_fails_closed() -> anyhow::Result<()> {
    // An inverted bf-range must not be accepted as a trustworthy mapping.
    let cmap = BASE_CMAP.replace(
        "1 beginbfchar\n<0001> <041F>\nendbfchar",
        "1 beginbfrange\n<0004> <0003> <4E16>\nendbfrange",
    );
    assert!(cmap.contains("<0004> <0003>"));
    let raw = fixture(FixtureFont::Type0Cmap(cmap.into_bytes()), cid_ops(), false)?;
    Transformer::parse(&Bytes::from(raw))
        .expect_err("inverted Unicode bf-range must not produce Complete text");
    Ok(())
}

#[test]
fn hostile_cmap_inflated_over_limit_fails_closed() -> anyhow::Result<()> {
    // A physically tiny Flate CMap can exceed the *decoded* 1-MiB budget.
    let mut cmap = BASE_CMAP.as_bytes().to_vec();
    cmap.extend(vec![b' '; MAX_TOUNICODE_CMAP + 256]);
    let raw = fixture(FixtureFont::Type0Cmap(cmap), cid_ops(), false)?;
    assert!(raw.len() < 128 * 1024, "CMap fixture must remain compressed");
    Transformer::parse(&Bytes::from(raw))
        .expect_err("decoded CMap above its limit must fail");
    Ok(())
}

#[test]
fn hostile_nested_tj_array_fails_closed() -> anyhow::Result<()> {
    let mut value = Object::string_literal("x");
    for _ in 0..=MAX_PDF_TEXT_ARRAY_DEPTH {
        value = Object::Array(vec![value]);
    }
    let raw = fixture(FixtureFont::Type1, latin_ops(vec![value]), false)?;
    let error = Transformer::parse(&Bytes::from(raw))
        .expect_err("over-deep PDF text array must be rejected");
    assert!(
        error.to_string().contains("nesting budget"),
        "unexpected nested-array rejection category"
    );
    Ok(())
}

#[test]
fn hostile_malformed_font_operand_fails_closed() -> anyhow::Result<()> {
    let raw = fixture(
        FixtureFont::Type1,
        vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Integer(1234), 12.into()]),
            Operation::new("Tj", vec![Object::string_literal("bad")]),
            Operation::new("ET", vec![]),
        ],
        false,
    )?;
    Transformer::parse(&Bytes::from(raw))
        .expect_err("Tf without a named font must fail, not invent a default");
    Ok(())
}

#[test]
fn hostile_undeclared_font_fails_closed() -> anyhow::Result<()> {
    let raw = fixture(
        FixtureFont::NoFont,
        latin_ops(vec![Object::string_literal("bad")]),
        false,
    )?;
    Transformer::parse(&Bytes::from(raw))
        .expect_err("text with undeclared F1 must fail");
    Ok(())
}

#[test]
fn hostile_dangling_contents_reference_fails_closed() -> anyhow::Result<()> {
    let raw = fixture(
        FixtureFont::Type1,
        latin_ops(vec![Object::string_literal("not-indexable")]),
        true,
    )?;
    Transformer::parse(&Bytes::from(raw))
        .expect_err("dangling /Contents object must not yield Complete");
    Ok(())
}

#[test]
fn hostile_unknown_stream_filter_fails_closed() -> anyhow::Result<()> {
    // lopdf's get_page_content_with_limit() can fall back to compressed
    // raw bytes on decode errors; Shiva must reject instead.
    let bytes = fixture(
        FixtureFont::Type1,
        latin_ops(vec![Object::string_literal("NotIndexable")]),
        false,
    )?;
    let mut doc = LopdfDocument::load_mem(&bytes)?;
    let page_id = *doc.get_pages().values().next()
        .expect("fixture must contain one page");
    let stream_id = *doc.get_page_contents(page_id).first()
        .expect("fixture must contain one content stream");
    let stream = doc.get_object_mut(stream_id)?.as_stream_mut()?;
    stream.dict.set("Filter", "DeliberatelyUnknownFilter");
    let mut broken = Vec::new();
    doc.save_to(&mut broken)?;

    Transformer::parse(&Bytes::from(broken))
        .expect_err("unsupported PDF filter must not fall back to raw data");
    Ok(())
}

#[test]
fn hostile_corrupt_flate_stream_fails_closed() -> anyhow::Result<()> {
    // A /Filter name can be syntactically valid while its compressed
    // bytes are invalid. Never treat raw data as indexed PDF operators.
    let bytes = fixture(
        FixtureFont::Type1,
        latin_ops(vec![Object::string_literal("NeverIndexRawDeflate")]),
        false,
    )?;
    let mut doc = LopdfDocument::load_mem(&bytes)?;
    let page_id = *doc.get_pages().values().next().unwrap();
    let stream_id = *doc.get_page_contents(page_id).first().unwrap();
    let stream = doc.get_object_mut(stream_id)?.as_stream_mut()?;
    stream.set_content(vec![0, 0, 0, 0, 0, 0, 0]);
    stream.dict.set("Filter", "FlateDecode");
    let mut broken = Vec::new();
    doc.save_to(&mut broken)?;
    Transformer::parse(&Bytes::from(broken))
        .expect_err("corrupt FlateDecode bytes must be rejected");
    Ok(())
}

#[test]
fn hostile_mixed_contents_array_member_fails_closed() -> anyhow::Result<()> {
    // lopdf's get_page_contents filters away non-reference array items.
    // The whole page must fail rather than appear partially extracted.
    let bytes = fixture(
        FixtureFont::Type1,
        latin_ops(vec![Object::string_literal("NeverPartiallyIndex")]),
        false,
    )?;
    let mut doc = LopdfDocument::load_mem(&bytes)?;
    let page_id = *doc.get_pages().values().next().unwrap();
    let stream_id = *doc.get_page_contents(page_id).first().unwrap();
    let page = doc.get_object_mut(page_id)?.as_dict_mut()?;
    page.set("Contents", Object::Array(vec![
        Object::Reference(stream_id), Object::Integer(123),
    ]));
    let mut broken = Vec::new();
    doc.save_to(&mut broken)?;
    Transformer::parse(&Bytes::from(broken))
        .expect_err("non-stream member in /Contents must not be silently ignored");
    Ok(())
}

#[test]
fn hostile_control_valid_blank_page_without_contents() -> anyhow::Result<()> {
    // A page genuinely lacking /Contents is a valid blank PDF page.
    let bytes = fixture(FixtureFont::NoFont, vec![], false)?;
    let mut doc = LopdfDocument::load_mem(&bytes)?;
    let page_id = *doc.get_pages().values().next().unwrap();
    doc.get_object_mut(page_id)?.as_dict_mut()?.remove(b"Contents");
    let mut blank = Vec::new();
    doc.save_to(&mut blank)?;
    let extracted = Transformer::parse(&Bytes::from(blank))?;
    let output = format!("{extracted:?}");
    assert!(!output.contains("Never"), "blank page unexpectedly contains text");
    Ok(())
}

#[test]
fn hostile_truncated_document_fails_closed() {
    let bytes = Bytes::from_static(b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\n");
    assert!(
        Transformer::parse(&bytes).is_err(),
        "truncated PDF without pages/xref must not produce a Document"
    );
}
