//! Strict, bounded PDF /Pages graph traversal for completeness-sensitive search.
//!
//! Unlike lopdf::Document::get_pages(), this validator never silently skips
//! invalid children, repeated page references or a truncated subtree. The
//! returned ordered list is the only list the extraction path may process.
//! Recovery for *viewing* is a separate concern and cannot claim Complete.
use anyhow::{bail, ensure, Result};
use lopdf::{Document as PdfDocument, Object, ObjectId};
use std::collections::BTreeSet;

use super::MAX_PDF_PAGES;

const MAX_TREE_DEPTH: usize = 64;
const MAX_TREE_NODES: usize = 4096;
const MAX_KIDS_INDIRECTION: usize = 16;

enum Step {
    Visit {
        id: ObjectId,
        parent: Option<ObjectId>,
        depth: usize,
    },
    FinishPages {
        declared_count: usize,
        leaves_before: usize,
    },
}

/// Return all pages in document /Kids order, or no pages at all on failure.
///
/// This validates the original object graph without mutation. Intermediate
/// /Pages counts are recalculated from traversed leaf pages (not trusted as an
/// oracle), and we never fall back to lopdf's recovery-oriented page iterator.
pub(super) fn validate_page_tree(pdf: &PdfDocument) -> Result<Vec<ObjectId>> {
    let root_id = pdf.catalog()?.get(b"Pages")?.as_reference()?;
    let mut result = Vec::new();
    let mut visited = BTreeSet::new();
    // /Kids itself may be an indirect array. It cannot be shared by two
    // branches without making ownership/completeness ambiguous.
    let mut used_kids_arrays = BTreeSet::new();
    let mut steps = vec![Step::Visit {
        id: root_id,
        parent: None,
        depth: 0,
    }];

    while let Some(step) = steps.pop() {
        match step {
            Step::FinishPages {
                declared_count,
                leaves_before,
            } => {
                let actual_count = result.len().checked_sub(leaves_before)
                    .ok_or_else(|| anyhow::anyhow!("PDF page-tree count underflow"))?;
                ensure!(
                    actual_count == declared_count,
                    "PDF page-tree /Count disagrees with visited pages"
                );
            }
            Step::Visit { id, parent, depth } => {
                ensure!(depth <= MAX_TREE_DEPTH, "PDF page-tree depth exceeded");
                ensure!(visited.insert(id), "PDF page-tree cycle or duplicate object");
                ensure!(visited.len() <= MAX_TREE_NODES, "PDF page-tree node budget exceeded");

                let dict = pdf.get_dictionary(id)?;
                match parent {
                    Some(expected_parent) => {
                        ensure!(
                            dict.get(b"Parent")?.as_reference()? == expected_parent,
                            "PDF page-tree /Parent mismatch"
                        );
                    }
                    None => {
                        ensure!(
                            dict.get(b"Parent").is_err(),
                            "PDF root /Pages must not have /Parent"
                        );
                    }
                }

                match dict.get(b"Type")?.as_name()? {
                    b"Page" => {
                        ensure!(parent.is_some(), "PDF root must be /Pages, not /Page");
                        ensure!(result.len() < MAX_PDF_PAGES, "PDF page count budget exceeded");
                        result.push(id);
                    }
                    b"Pages" => {
                        let declared = dict.get(b"Count")?.as_i64()?;
                        ensure!(
                            declared > 0 && declared as u64 <= MAX_PDF_PAGES as u64,
                            "PDF page-tree /Count exceeds allowed positive range"
                        );

                        let mut raw_kids = dict.get(b"Kids")?;
                        let mut references = 0usize;
                        while let Object::Reference(kids_id) = raw_kids {
                            references = references.checked_add(1)
                                .ok_or_else(|| anyhow::anyhow!("PDF /Kids chain overflow"))?;
                            ensure!(
                                references <= MAX_KIDS_INDIRECTION,
                                "PDF /Kids indirection budget exceeded"
                            );
                            ensure!(
                                used_kids_arrays.insert(*kids_id),
                                "PDF /Kids indirect array is cyclic or shared"
                            );
                            raw_kids = pdf.get_object(*kids_id)?;
                        }
                        let kids = raw_kids.as_array()?;
                        ensure!(!kids.is_empty(), "PDF /Pages has no /Kids");
                        ensure!(
                            kids.len() <= MAX_TREE_NODES,
                            "PDF /Kids fan-out exceeds node budget"
                        );
                        let child_depth = depth.checked_add(1)
                            .ok_or_else(|| anyhow::anyhow!("PDF page-tree depth overflow"))?;
                        ensure!(child_depth <= MAX_TREE_DEPTH, "PDF page-tree depth exceeded");

                        steps.push(Step::FinishPages {
                            declared_count: declared as usize,
                            leaves_before: result.len(),
                        });
                        // Reverse push yields exact original /Kids order.
                        for kid in kids.iter().rev() {
                            let child_id = kid.as_reference().map_err(|_| {
                                anyhow::anyhow!(
                                    "PDF /Kids must contain indirect /Page or /Pages references"
                                )
                            })?;
                            steps.push(Step::Visit {
                                id: child_id,
                                parent: Some(id),
                                depth: child_depth,
                            });
                        }
                    }
                    _ => bail!("PDF page-tree node has invalid /Type"),
                }
            }
        }
    }

    ensure!(!result.is_empty(), "PDF page tree has no pages");
    Ok(result)
}

