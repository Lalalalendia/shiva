//! Bounded deterministic mutation QA for **only** pinned external PDF samples.
//! This is not coverage-guided fuzzing. It asserts a semantic invariant:
//! structurally changed PDFs must either reject OR preserve every expected
//! original text-page marker in correct order. No success with partial text.
//! Mutations exist in memory only and never touch source files.
#![cfg(feature = "qa-external-pdf")]

use bytes::Bytes;
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_HITS_PER_TOKEN: usize = 6;
// ONLY keys required by the PDF page graph or source container. Optional
// /Contents, /Resources, /MediaBox and /Filter may be legitimately absent;
// a renamed optional key can produce a valid blank page, which is not
// distinguishable from intentional blank content by a PDF parser alone.
// Such mutations require trusted snapshot identity, not a false completeness
// claim based on bytes from a different source revision.
const REQUIRED_GRAPH_TOKENS: &[&[u8]] = &[
    b"/Kids", b"/Pages", b"/Page", b"/Parent", b"/Count",
    b"/Root", b"startxref", b"trailer", b"/Type",
];

#[derive(Clone, Copy)]
enum Source {
    PdfLib,
    ReportLab,
}

struct Seed {
    name: &'static str,
    source: Source,
    pages: usize,
}

const SEEDS: &[Seed] = &[
    Seed { name: "sample-1-page.pdf", source: Source::PdfLib, pages: 1 },
    Seed { name: "sample-10-pages.pdf", source: Source::PdfLib, pages: 10 },
    Seed { name: "reportlab-12-pages.pdf", source: Source::ReportLab, pages: 12 },
    Seed { name: "reportlab-rotated-3-pages.pdf", source: Source::ReportLab, pages: 3 },
];

fn directory(var: &str) -> PathBuf {
    let root = PathBuf::from(std::env::var_os(var)
        .unwrap_or_else(|| panic!("QA path missing: {var}")));
    assert!(root.is_absolute() && root.is_dir(), "missing verified QA directory");
    root
}

fn seed_bytes(seed: &Seed, lib: &Path, reportlab: &Path) -> Vec<u8> {
    let root = match seed.source {
        Source::PdfLib => lib,
        Source::ReportLab => reportlab,
    };
    let bytes = std::fs::read(root.join(seed.name)).expect("pinned QA seed missing");
    assert!(
        (100..=MAX_FILE_BYTES).contains(&bytes.len()) && bytes.starts_with(b"%PDF-"),
        "invalid or oversized QA seed"
    );
    bytes
}

fn marker(seed: &Seed, page: usize) -> String {
    match seed.source {
        Source::PdfLib => format!("Page {page} of {}", seed.pages),
        Source::ReportLab => format!("RLQA-{page:03}-OF-{:03}", seed.pages),
    }
}

fn exactly_all_original_text_pages(output: &str, seed: &Seed, mutation: &str) {
    let mut previous_start: Option<usize> = None;
    for page in 1..=seed.pages {
        let expected = marker(seed, page);
        let hits = output.matches(&expected).count();
        assert_eq!(
            hits, 1,
            "partial/duplicate text after {mutation} in {}: page {page} of {}",
            seed.name, seed.pages
        );
        let at = output.find(&expected).expect("unique marker must be present");
        if let Some(prev) = previous_start {
            assert!(
                at > prev,
                "wrong page order after {mutation} in {}: page {page}",
                seed.name
            );
        }
        previous_start = Some(at);
    }
}

fn parse_with_oracle(
    bytes: Vec<u8>,
    seed: &Seed,
    mutation: &str,
    expected_complete: Option<&str>,
) -> Option<String> {
    // A parser panic is a test failure, never a successful rejection.
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        Transformer::parse(&Bytes::from(bytes))
    }))
    .unwrap_or_else(|_| panic!("PDF mutation caused parser panic: {} {mutation}", seed.name));

    match parsed {
        Err(_) => None, // Explicit rejection is acceptable.
        Ok(doc) => {
            let output = format!("{doc:?}");
            exactly_all_original_text_pages(&output, seed, mutation);
            if let Some(expected) = expected_complete {
                // Required graph-key mutations cannot alter original body text.
                // Page markers alone miss deleted words inside intact pages.
                // Never print document contents in CI if the comparison fails.
                assert!(
                    output == expected,
                    "PDF semantic text drift after {mutation} in {} despite intact page markers",
                    seed.name
                );
            }
            Some(output)
        }
    }
}

fn positions<'a>(data: &'a [u8], token: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    data.windows(token.len())
        .enumerate()
        .filter(move |(_, slice)| *slice == token)
        .map(|(index, _)| index)
        .take(MAX_HITS_PER_TOKEN)
}

fn flip_ascii_byte(value: u8) -> u8 {
    if value == b'Z' { b'Y' } else { b'Z' }
}

/// Mutate required page-graph keys and numeric /Count only. Optional stream
/// and resource entries are deliberately excluded: removing /Contents can
/// transform a text page into a *valid blank PDF page*, a fundamentally
/// ambiguous changed revision without an independent source oracle.
#[test]
fn mutated_required_page_graph_cannot_silently_lose_text_pages() {
    let pdf_lib = directory("SHIVA_QA_PINNED_PDF_DIR");
    let reportlab = directory("SHIVA_QA_REPORTLAB_DIR");
    let mut candidates = 0usize;
    let mut rejected = 0usize;
    let mut accepted_complete = 0usize;

    for seed in SEEDS {
        let original = seed_bytes(seed, &pdf_lib, &reportlab);
        let baseline = parse_with_oracle(
            original.clone(), seed, "unmodified baseline", None,
        ).expect("known-good independent PDF seed must parse");
        for token in REQUIRED_GRAPH_TOKENS {
            for start in positions(&original, token) {
                let mut altered = original.clone();
                // Keep the / separator intact, changing just one letter.
                let offset = start + token.len() - 1;
                altered[offset] = flip_ascii_byte(altered[offset]);
                let description = format!(
                    "key-token-{}-at-{}", String::from_utf8_lossy(token), start
                );
                candidates += 1;
                if parse_with_oracle(altered, seed, &description, Some(&baseline)).is_some() {
                    accepted_complete += 1;
                } else {
                    rejected += 1;
                }
            }
        }

        // Correct-looking but wrong numeric /Count must not shorten the index.
        let token = b"/Count";
        for start in positions(&original, token) {
            let mut cursor = start + token.len();
            while cursor < original.len() && original[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < original.len() && original[cursor].is_ascii_digit() {
                let mut altered = original.clone();
                altered[cursor] = if altered[cursor] == b'9' { b'8' } else { b'9' };
                candidates += 1;
                let description = format!("count-digit-at-{cursor}");
                if parse_with_oracle(altered, seed, &description, Some(&baseline)).is_some() {
                    accepted_complete += 1;
                } else {
                    rejected += 1;
                }
            }
        }
    }
    assert!(candidates >= 50, "mutation corpus unexpectedly lost required-graph coverage");
    assert_eq!(candidates, rejected + accepted_complete);
    println!(
        "SHIVA_QA_SEMANTIC_MUTATION=pass;cases={candidates};reject={rejected};complete={accepted_complete};sources=2;scope=required_page_graph"
    );
}
