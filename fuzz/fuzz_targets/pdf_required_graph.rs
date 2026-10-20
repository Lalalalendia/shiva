#![no_main]
// Coverage-guided semantic PDF fuzz target. The corpus is a tiny *instruction*
// stream; the mutated document is reconstructed in memory from immutable,
// independently pinned CI seeds. Optional /Contents and page resources are
// explicitly out of scope: removing them can legally create a blank PDF page.
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;
use std::path::PathBuf;
use std::sync::OnceLock;

const MAX_PDF_BYTES: usize = 256 * 1024;
const MAX_INSTRUCTION: usize = 16;
const KEYWORDS: &[&[u8]] = &[
    b"/Kids", b"/Pages", b"/Page", b"/Parent", b"/Count",
    b"/Root", b"/Type", b"startxref", b"trailer",
];

struct Baseline {
    name: &'static str,
    bytes: Vec<u8>,
    complete: String,
}

fn validate_markers(output: &str, generator: usize, n: usize) {
    let mut previous = None;
    for page in 1..=n {
        let marker = if generator == 0 {
            format!("Page {page} of {n}")
        } else {
            format!("RLQA-{page:03}-OF-{n:03}")
        };
        assert_eq!(
            output.matches(&marker).count(), 1,
            "fuzz source has missing or duplicated marker at page {page} of {n}"
        );
        let current = output.find(&marker).expect("page marker must exist");
        if let Some(last) = previous {
            assert!(current > last, "source pages changed order");
        }
        previous = Some(current);
    }
}

fn read_baseline(env_name: &str, file: &'static str, generator: usize, pages: usize) -> Baseline {
    let dir = PathBuf::from(
        std::env::var_os(env_name).expect("pinned QA seed directory not supplied")
    );
    assert!(dir.is_absolute() && dir.is_dir(), "missing exact QA seed directory");
    let bytes = std::fs::read(dir.join(file)).expect("pinned PDF seed not provisioned");
    assert!(
        (100..=MAX_PDF_BYTES).contains(&bytes.len()) && bytes.starts_with(b"%PDF-"),
        "fuzz PDF input size/header violation"
    );
    let parsed = Transformer::parse(&Bytes::from(bytes.clone()))
        .expect("pinned original PDF must parse before mutation");
    let complete = format!("{parsed:?}");
    validate_markers(&complete, generator, pages);
    Baseline { name: file, bytes, complete }
}

static BASELINES: OnceLock<Vec<Baseline>> = OnceLock::new();

fn sources() -> &'static [Baseline] {
    BASELINES.get_or_init(|| vec![
        read_baseline("SHIVA_QA_PINNED_PDF_DIR", "sample-1-page.pdf", 0, 1),
        read_baseline("SHIVA_QA_PINNED_PDF_DIR", "sample-10-pages.pdf", 0, 10),
        read_baseline("SHIVA_QA_REPORTLAB_DIR", "reportlab-12-pages.pdf", 1, 12),
        read_baseline("SHIVA_QA_REPORTLAB_DIR", "reportlab-rotated-3-pages.pdf", 1, 3),
    ])
}

fn locations(bytes: &[u8], token: &[u8]) -> Vec<usize> {
    bytes.windows(token.len()).enumerate()
        .filter(|(_, window)| *window == token)
        .map(|(i, _)| i)
        .take(24)
        .collect()
}

fuzz_target!(|instruction: &[u8]| {
    if instruction.len() < 4 || instruction.len() > MAX_INSTRUCTION {
        return;
    }
    let seeds = sources();
    let source = &seeds[instruction[0] as usize % seeds.len()];
    let keyword = KEYWORDS[instruction[1] as usize % KEYWORDS.len()];
    let hits = locations(&source.bytes, keyword);
    if hits.is_empty() {
        return;
    }
    let position = hits[instruction[2] as usize % hits.len()];
    let mut modified = source.bytes.clone();

    if keyword == b"/Count" && instruction[3] & 1 == 1 {
        // Keep valid numeric syntax and unchanged xref offsets while
        // challenging the claimed number of descendant /Page objects.
        let mut digit = position + keyword.len();
        while digit < modified.len() && modified[digit].is_ascii_whitespace() {
            digit += 1;
        }
        if digit >= modified.len() || !modified[digit].is_ascii_digit() {
            return;
        }
        modified[digit] = if modified[digit] == b'9' { b'8' } else { b'9' };
    } else {
        // Change one required graph-key byte, retaining the same length.
        let offset = position + keyword.len() - 1;
        modified[offset] = if modified[offset] == b'Z' { b'Y' } else { b'Z' };
    }

    // A libFuzzer crash or panic is a real QA failure, not an acceptable
    // parser rejection. For every accepted mutation, require *all* text,
    // ordering and multiplicity to equal the pinned original's extraction.
    match Transformer::parse(&Bytes::from(modified)) {
        Err(_) => {},
        Ok(doc) => {
            let result = format!("{doc:?}");
            assert!(
                result == source.complete,
                "silent extracted-text drift after required page-graph mutation in {}",
                source.name
            );
        }
    }
});
