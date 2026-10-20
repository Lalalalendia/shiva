#![no_main]
// CI-only libFuzzer target: completely separate from the shipping Shiva worker.
// Deliberately run full parser entrypoint without catching panics: crashes are bugs.
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;

const MAX_INPUT: usize = 256 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() <= MAX_INPUT {
        let _ = Transformer::parse(&Bytes::copy_from_slice(data));
    }
});
