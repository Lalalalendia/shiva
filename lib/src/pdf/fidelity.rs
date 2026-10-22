//! Conservative diagnostic contract for the native PDF text parser.
//!
//! A successful native text decode is not a proof of painted-page fidelity.
//! These statuses are intentionally NOT named Complete, even on clean PDFs.
//! Future visual verification and Akula index/file authorization require
//! separate end-to-end acceptance; this data never relaxes worker sandboxing.

use crate::core::Document;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfFidelityStatus {
    /// Native text was extracted, but an independent visual oracle did not run.
    NeedsReview,
    /// Some page graphics may contain content the native parser skipped.
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfFidelityIssue {
    /// Every page still lacks independently verified painted-text fidelity.
    VisualFidelityUnverified,
    /// A Do operator was present, but Image/Form resources were not traversed.
    UnprocessedXObject,
    /// Rendering mode 3 was observed. Do not assume it is malicious: the PDF
    /// may deliberately store a legitimate invisible OCR text layer.
    InvisibleTextRenderModeObserved,
}

#[derive(Debug)]
pub struct PdfPageFidelity {
    /// One-based page index from the fully validated page tree.
    pub page_number: usize,
    /// Unique, bounded diagnostic reasons (never user content or PDF names).
    pub issues: Vec<PdfFidelityIssue>,
}

impl PdfPageFidelity {
    pub(super) fn unverified(page_number: usize) -> Self {
        Self {
            page_number,
            issues: vec![PdfFidelityIssue::VisualFidelityUnverified],
        }
    }

    pub(super) fn mark(&mut self, issue: PdfFidelityIssue) {
        if !self.issues.contains(&issue) {
            self.issues.push(issue);
        }
    }
}

#[derive(Debug)]
pub struct PdfExtractionReport {
    /// Provisional text; consumers must inspect status before indexing or
    /// presenting it as complete. The older parse() API cannot carry status.
    pub document: Document,
    pub status: PdfFidelityStatus,
    pub pages: Vec<PdfPageFidelity>,
}

impl PdfExtractionReport {
    pub(super) fn new(document: Document, pages: Vec<PdfPageFidelity>) -> Self {
        let status = if pages
            .iter()
            .any(|page| page.issues.contains(&PdfFidelityIssue::UnprocessedXObject))
        {
            PdfFidelityStatus::Partial
        } else {
            PdfFidelityStatus::NeedsReview
        };
        Self {
            document,
            status,
            pages,
        }
    }
}
