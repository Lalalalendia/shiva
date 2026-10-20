// Separate Windows OS-process experiment; ignored outside dedicated CI.
// It does not enable PDF in the actual shiva-worker format allowlist.
#![cfg(all(windows, debug_assertions, feature = "qa-pdf-windows"))]

#[test]
#[ignore = "requires hosted Windows CI with an exact LPAC SID ACL fixture"]
fn restricted_pdf_qa_process_has_lpac_job_and_memory_evidence() {
    let executable = std::path::PathBuf::from(
        std::env::var_os("SHIVA_TEST_PDF_QA_BIN")
            .expect("CI-only synthetic PDF example path must be provided"),
    );
    assert!(executable.is_absolute() && executable.is_file(),
        "Windows CI must build the exact synthetic PDF executable");
    let (smoke, cumulative) =
        shiva_worker_client::windows_pdf_resource_qa(&executable)
            .expect("PDF subprocess must pass attested AppContainer + atomic Job gate");
    const JOB_MAX: usize = 512 * 1024 * 1024;
    assert!(smoke > 0 && smoke <= JOB_MAX);
    assert!(cumulative > 0 && cumulative <= JOB_MAX);
    eprintln!("SHIVA_WIN_PDF_SMOKE_JOB_PEAK_BYTES={smoke}");
    eprintln!("SHIVA_WIN_PDF_CUMULATIVE_JOB_PEAK_BYTES={cumulative}");
    eprintln!("SHIVA_WIN_PDF_LPAC_JOB_RESOURCE_QA=pass");
}
