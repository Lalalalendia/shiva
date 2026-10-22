//! Bounded native-text extraction from PDF Form XObjects.
//!
//! Only text in a referenced /Form is collected. Images, missing resources,
//! cycles and exhausted budgets stay Partial. No image decoding/OCR, no worker
//! admission and no inference of full painted-page fidelity.
use super::{
    validate_flate_integrity, PdfBudget, PdfFidelityIssue, PdfPageFidelity,
    MAX_DECOMPRESSED_STREAM, MAX_TOUNICODE_CMAP,
};
use anyhow::{ensure, Result};
use lopdf::content::Content;
use lopdf::{Dictionary, Document as PdfDocument, Encoding, Object, ObjectId};
use std::collections::BTreeMap;

const MAX_FORM_DEPTH: usize = 4;
const MAX_FORM_INVOCATIONS: usize = 128;
const MAX_FORM_RESOURCES: usize = 128;
const MAX_FORM_FONTS: usize = 64;
const MAX_FORM_FONT_DECODES_PER_PAGE: usize = 128;

#[derive(Default)]
pub(super) struct FormBudget {
    invocations: usize,
    font_decodes: usize,
}

fn resources_for_page<'a>(
    pdf: &'a PdfDocument,
    page_id: ObjectId,
) -> Result<Option<&'a Dictionary>> {
    // The PDF /Resources entry can be directly on /Page or inherited from
    // its /Pages ancestors. The existing page-tree validator runs first.
    let (direct, indirect_ids) = pdf.get_page_resources(page_id)?;
    if let Some(resources) = direct {
        return Ok(Some(resources));
    }
    if let Some(id) = indirect_ids.first() {
        return Ok(Some(pdf.get_dictionary(*id)?));
    }
    Ok(None)
}

fn decoded_fonts<'a>(
    pdf: &'a PdfDocument,
    resources: &'a Dictionary,
) -> Result<BTreeMap<Vec<u8>, Encoding<'a>>> {
    let mut result = BTreeMap::new();
    if let Ok(fonts) = pdf.get_dict_in_dict(resources, b"Font") {
        ensure!(fonts.len() <= MAX_FORM_FONTS, "PDF Form font count budget exceeded");
        for (name, value) in fonts.iter() {
            let (_, value) = pdf.dereference(value)?;
            let font = value.as_dict()?;
            let composite = font.get(b"Subtype")
                .and_then(Object::as_name)
                .map(|subtype| subtype == b"Type0")
                .unwrap_or(false);
            let encoding = font.get_font_encoding_with_limit(
                pdf, MAX_TOUNICODE_CMAP,
            )?;
            ensure!(
                !composite || matches!(&encoding, Encoding::UnicodeMapEncoding(_)),
                "PDF Form composite font has no usable ToUnicode mapping"
            );
            result.insert(name.to_vec(), encoding);
        }
    }
    Ok(result)
}

fn line_boundary(text: &mut String, budget: &mut PdfBudget) -> Result<()> {
    if !text.is_empty() && !text.ends_with('\n') {
        budget.text(1)?;
        text.push('\n');
    }
    Ok(())
}

fn collect_text(
    out: &mut String,
    encoding: &Encoding<'_>,
    operands: &[Object],
    budget: &mut PdfBudget,
    depth: usize,
) -> Result<()> {
    ensure!(depth <= super::MAX_PDF_TEXT_ARRAY_DEPTH, "PDF Form text nesting exceeded");
    for operand in operands {
        match operand {
            Object::String(raw, _) => {
                let decoded = PdfDocument::decode_text(encoding, raw)?;
                ensure!(!decoded.contains('\u{FFFD}'),
                    "PDF Form glyph could not be mapped to Unicode");
                budget.text(decoded.len())?;
                out.push_str(&decoded);
            }
            Object::Array(children) => {
                collect_text(out, encoding, children, budget, depth + 1)?;
                budget.text(1)?;
                out.push(' ');
            }
            Object::Integer(value) if *value < -100 => {
                budget.text(1)?;
                out.push(' ');
            }
            _ => {}
        }
    }
    Ok(())
}

/// A page's Do uses the page resource scope, not the global PDF object table.
/// Its parsed text is provisional and may only be appended at the Do position.
pub(super) fn from_page(
    pdf: &PdfDocument,
    page_id: ObjectId,
    name: &[u8],
    form_budget: &mut FormBudget,
    budget: &mut PdfBudget,
    fidelity: &mut PdfPageFidelity,
) -> Result<Option<String>> {
    let Some(resources) = resources_for_page(pdf, page_id)? else {
        fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
        return Ok(None);
    };
    visit(pdf, resources, name, form_budget, budget, fidelity, &mut Vec::new())
}

fn visit<'a>(
    pdf: &'a PdfDocument,
    resources: &'a Dictionary,
    name: &[u8],
    form_budget: &mut FormBudget,
    budget: &mut PdfBudget,
    fidelity: &mut PdfPageFidelity,
    active: &mut Vec<ObjectId>,
) -> Result<Option<String>> {
    if active.len() >= MAX_FORM_DEPTH || form_budget.invocations >= MAX_FORM_INVOCATIONS {
        fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
        return Ok(None);
    }
    let Ok(xobjects) = pdf.get_dict_in_dict(resources, b"XObject") else {
        fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
        return Ok(None);
    };
    ensure!(xobjects.len() <= MAX_FORM_RESOURCES,
        "PDF Form XObject resource count budget exceeded");
    let Ok(raw) = xobjects.get(name) else {
        fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
        return Ok(None);
    };
    // Valid PDF streams are indirect. This also gives a reliable cycle key.
    let Object::Reference(id) = raw else {
        fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
        return Ok(None);
    };
    if active.contains(id) {
        fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
        return Ok(None);
    }
    let stream = pdf.get_object(*id)?.as_stream()?;
    match stream.dict.get(b"Subtype").and_then(Object::as_name) {
        Ok(b"Form") => {}
        // Images may contain visible words that are not PDF text operators.
        _ => {
            fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
            return Ok(None);
        }
    }
    // This object is a supported Form. Charge every invocation, including
    // repeat paints of the same object with different transforms.
    form_budget.invocations += 1;
    active.push(*id);

    let form_resources = match stream.dict.get(b"Resources") {
        Ok(value) => {
            let (_, resolved) = pdf.dereference(value)?;
            match resolved {
                Object::Dictionary(dict) => dict,
                _ => {
                    fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
                    active.pop();
                    return Ok(None);
                }
            }
        }
        // The resource scope of a form without a new dictionary is inherited
        // only from its caller, never guessed from unrelated pages.
        Err(_) => resources,
    };
    let filters = if stream.dict.get(b"Filter").is_ok() {
        stream.filters()?
    } else {
        Vec::new()
    };
    if filters.len() == 1 && filters[0] == b"FlateDecode" {
        validate_flate_integrity(&stream.content, MAX_DECOMPRESSED_STREAM)?;
    }
    let bytes = stream.get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM)?;
    ensure!(bytes.len() <= MAX_DECOMPRESSED_STREAM,
        "PDF Form decoded stream exceeds per-stream limit");
    budget.page_content(bytes.len())?;
    let decoded = Content::decode(&bytes)?;
    budget.operations(decoded.operations.len())?;
    // A small Form can be painted many times. Per-Form limits are not
    // sufficient if font CMaps are decoded again on every invocation.
    let font_count = pdf.get_dict_in_dict(form_resources, b"Font")
        .map(|fonts| fonts.len()).unwrap_or(0);
    let next = form_budget.font_decodes.saturating_add(font_count);
    if next > MAX_FORM_FONT_DECODES_PER_PAGE {
        fidelity.mark(PdfFidelityIssue::UnprocessedXObject);
        active.pop();
        return Ok(None);
    }
    form_budget.font_decodes = next;
    let fonts = decoded_fonts(pdf, form_resources)?;
    let mut current: Option<&Encoding<'_>> = None;
    let mut out = String::new();
    for op in &decoded.operations {
        match op.operator.as_ref() {
            "Tf" => {
                let font = op.operands.first()
                    .ok_or_else(|| anyhow::anyhow!("PDF Form Tf missing font"))?
                    .as_name()?;
                current = Some(fonts.get(font)
                    .ok_or_else(|| anyhow::anyhow!("PDF Form font unavailable"))?);
            }
            "Tj" | "TJ" => {
                let font = current.ok_or_else(|| anyhow::anyhow!("PDF Form text without font"))?;
                collect_text(&mut out, font, &op.operands, budget, 0)?;
            }
            "Td" | "TD" => {
                ensure!(op.operands.len() == 2 &&
                    op.operands.iter().all(|x| matches!(x, Object::Integer(_) | Object::Real(_))),
                    "PDF Form text move requires two numbers");
                if match &op.operands[1] {
                    Object::Integer(y) => *y != 0,
                    Object::Real(y) => *y != 0.0,
                    _ => false,
                } {
                    line_boundary(&mut out, budget)?;
                }
            }
            "T*" => {
                ensure!(op.operands.is_empty(), "PDF Form T* takes no operands");
                line_boundary(&mut out, budget)?;
            }
            "'" | "\"" => {
                let count = if op.operator == "'" { 1 } else { 3 };
                ensure!(op.operands.len() == count,
                    "PDF Form quote has invalid operand count");
                if count == 3 {
                    ensure!(op.operands[..2].iter()
                        .all(|x| matches!(x, Object::Integer(_) | Object::Real(_))),
                        "PDF Form quote spacing requires numbers");
                }
                let value = op.operands.last()
                    .ok_or_else(|| anyhow::anyhow!("PDF Form quote missing text"))?;
                ensure!(matches!(value, Object::String(_, _)),
                    "PDF Form quote must contain a string");
                let font = current.ok_or_else(|| anyhow::anyhow!("PDF Form quote without font"))?;
                line_boundary(&mut out, budget)?;
                collect_text(&mut out, font, std::slice::from_ref(value), budget, 0)?;
            }
            "Tm" | "ET" => line_boundary(&mut out, budget)?,
            "Tr" => {
                if matches!(op.operands.first(), Some(Object::Integer(3))) {
                    fidelity.mark(PdfFidelityIssue::InvisibleTextRenderModeObserved);
                }
            }
            "Do" => {
                ensure!(op.operands.len() == 1, "PDF Form Do requires one name");
                let child_name = op.operands[0].as_name()?;
                if let Some(child) = visit(
                    pdf, form_resources, child_name, form_budget, budget, fidelity, active,
                )? {
                    if !child.is_empty() {
                        line_boundary(&mut out, budget)?;
                        // Child's bytes were charged during the recursive walk.
                        out.push_str(&child);
                    }
                }
            }
            // Remaining operators can affect geometry or non-text graphics;
            // we never claim painted-page completeness based on text alone.
            _ => {}
        }
    }
    active.pop();
    Ok(Some(out))
}
