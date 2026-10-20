// CI-only executable: deliberately NOT part of shiva-worker.
// Invoked by a separate Linux child process under prlimit + timeout;
// emits constant status codes, never PDF text, paths or input bytes.
use bytes::Bytes;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as LopdfDocument, Object, Stream};
use shiva::core::TransformerTrait;
use shiva::pdf::Transformer;

fn fixture(aggregate: bool) -> anyhow::Result<Vec<u8>> {
    let mut pdf = LopdfDocument::with_version("1.5");
    let pages_id = pdf.new_object_id();
    let font_id = pdf.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id = pdf.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let mut page_ids = Vec::new();
    let page_count = if aggregate { 5 } else { 1 };
    for _ in 0..page_count {
        let mut stream = if aggregate {
            // Five individually legal 4-MiB page streams exceed the
            // cumulative 16-MiB decoded-page ceiling in the next PR.
            Stream::new(dictionary! {}, vec![b' '; 4 * 1024 * 1024 + 64])
        } else {
            let ops = Content { operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("HelloBounded")]),
                Operation::new("ET", vec![]),
            ] };
            Stream::new(dictionary! {}, ops.encode()?)
        };
        stream.compress()?;
        let stream_id = pdf.add_object(stream);
        let page = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => resources_id,
            "Contents" => stream_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        page_ids.push(page);
    }
    pdf.objects.insert(pages_id, Object::Dictionary(dictionary! {
        "Type" => "Pages",
        "Count" => page_ids.len() as i64,
        "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
    }));
    let catalog = pdf.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    pdf.trailer.set("Root", catalog);
    let mut output = Vec::new();
    pdf.save_to(&mut output)?;
    Ok(output)
}

fn run(mode: &str) -> anyhow::Result<()> {
    let aggregate = match mode {
        "smoke" => false,
        "cumulative" => true,
        _ => anyhow::bail!("invalid scenario"),
    };
    let bytes = fixture(aggregate)?;
    anyhow::ensure!(bytes.len() < 1024 * 1024, "fixture not compressed");
    let result = Transformer::parse(&Bytes::from(bytes));
    if aggregate {
        let error = result.expect_err("aggregate budget bypass");
        anyhow::ensure!(
            error.to_string().contains("PDF cumulative decoded page budget"),
            "not rejected by the intended cumulative limit"
        );
    } else {
        let doc = result?;
        let extracted = format!("{doc:?}");
        anyhow::ensure!(
            extracted.matches("HelloBounded").count() == 1,
            "incorrect smoke extraction"
        );
    }
    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(|_| eprintln!("SHIVA_PDF_QA_CHILD_PANIC")));
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 2 || !matches!(args[1].as_str(), "smoke" | "cumulative") {
        eprintln!("SHIVA_PDF_QA_UNSUPPORTED_SCENARIO");
        std::process::exit(2);
    }
    if run(&args[1]).is_err() {
        // Deliberately do not print parser errors: even errors in real PDFs
        // might include document metadata or paths in future versions.
        eprintln!("SHIVA_PDF_QA_CHILD_FAILED");
        std::process::exit(3);
    }
    println!("SHIVA_PDF_QA_CHILD_OK");
}
