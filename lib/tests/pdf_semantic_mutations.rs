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
const MAX_HITS_PER_TOKEN: usize = 3;
const STRUCTURAL_TOKENS: &[&[u8]] = &[
    b"/Kids", b"/Pages", b"/Page", b"/Parent", b"/Count",
    b"/Contents", b"/Resources", b"/MediaBox", b"/Filter",
    b"/Length", b"/Root", b"startxref", b"trailer",
    b"/Type", b"/Rotate",
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

fn parse_with_oracle(bytes: Vec<u8>, seed: &Seed, mutation: &str) -> bool {
    // A parser panic is a *test failure*, not an acceptable reject.
    let parsed = catch_unwind(AssertUnwindSafe(|| {
        Transformer::parse(&Bytes::from(bytes))
    }))
    .unwrap_or_else(|_| panic!("PDF mutation caused parser panic: {} {mutation}", seed.name));

    match parsed {
        Err(_) => false, // Explicit rejection is acceptable.
        Ok(doc) => {
            let output = format!("{doc:?}");
            exactly_all_original_text_pages(&output, seed, mutation);
            true
        }
    }
}

fn positions(data: &[u8], token: &[u8]) -> impl Iterator<Item = usize> + '_ {
    data.windows(token.len())
        .enumerate()
        .filter(move |(_, slice)| *slice == token)
        .map(|(index, _)| index)
        .take(MAX_HITS_PER_TOKEN)
}

fn flip_ascii_byte(value: u8) -> u8 {
    if value == b'Z' { b'Y' } else { b'Z' }
}

/// Restrict byte-level mutations to structural key tokens and their scalar
/// /Count values, not arbitrary text content. A valid successful parse must
/// consequently preserve all original page text: the source was never edited.
#[test]
fn mutated_page_structures_cannot_silently_lose_text_pages() {
    let pdf_lib = directory("SHIVA_QA_PINNED_PDF_DIR");
    let reportlab = directory("SHIVA_QA_REPORTLAB_DIR");
    let mut candidates = 0usize;
    let mut rejected = 0usize;
    let mut accepted_complete = 0usize;

    for seed in SEEDS {
        let original = seed_bytes(seed, &pdf_lib, &reportlab);
        assert!(
            parse_with_oracle(original.clone(), seed, "unmodified baseline"),
            "known-good independent PDF seed must parse"
        );
        for token in STRUCTURAL_TOKENS {
            for start in positions(&original, token) {
                let mut altered = original.clone();
                // Keep the / separator intact, changing just one letter.
                let offset = start + token.len() - 1;
                altered[offset] = flip_ascii_byte(altered[offset]);
                let description = format!(
                    "key-token-{}-at-{}", String::from_utf8_lossy(token), start
                );
                candidates += 1;
                if parse_with_oracle(altered, seed, &description) {
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
                if parse_with_oracle(altered, seed, &description) {
                    accepted_complete += 1;
                } else {
                    rejected += 1;
                }
            }
        }
    }
    assert!(candidates >= 80, "mutation corpus unexpectedly lost structural coverage");
    assert_eq!(candidates, rejected + accepted_complete);
    println!(
        "SHIVA_QA_SEMANTIC_MUTATION=pass;cases={candidates};reject={rejected};complete={accepted_complete};sources=2"
    );
}
