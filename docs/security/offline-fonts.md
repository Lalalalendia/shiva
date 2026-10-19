# Offline PDF / font policy for the Akula-facing Shiva fork

Upstream Shiva 1.4.9 automatically created `./fonts` and contacted GitHub to
download fonts when generating a PDF. This is not an acceptable default for
a private/local vault and makes ordinary `Document::generate(PDF)` depend on
network access and mutable working directory state.

The P2-MFORMAT-03b change removes network fetches and implicit file writes:
- `ShivaWorld::new(...)` is now fallible (`anyhow::Result<Self>`).
- Fonts must already exist in `./fonts`, or a trusted host can provide
  `SHIVA_FONT_DIR`. `with_font_dir(..., &Path)` is preferable for callers
  requiring an explicit capability boundary.
- Only regular TTF/OTF/TTC files are read; symlinked files are ignored.
  The scan has entry, per-file, total-byte and per-collection face limits.
- Missing/invalid fonts and Typst diagnostics return errors instead of
  initiating downloads or panicking. The PDF generator uses the fallible API.
- Bundled sample fonts in `lib/fonts` support the repository's PDF tests
  when invoked in the crate's working directory. CLI invocations from the
  workspace root should set `SHIVA_FONT_DIR=lib/fonts`.

This is a **breaking public API change** to `ShivaWorld::new` in our fork,
not published as a Shiva release. Existing privileged consumers should not
adopt it until compatibility tests and versioning are complete.

Scope deliberately does not claim all of Shiva is offline: other handlers
may access the filesystem or URLs supplied to them. This PR does not
establish a sandbox, does not provide a cryptographic font supply chain,
and cannot prevent symlink races in caller-controlled font directory trees.
Production Akula must run extraction in a no-network, OS-restricted subprocess,
with explicitly authorized bytes and limits for fonts, archives and images.

The offline-font CI checks minimal PDF output separately from the existing
hyperlink PDF regression, both using the bundled fonts via
`SHIVA_FONT_DIR=lib/fonts`. This is a focused smoke test, not a visual
rendering comparison or an assertion that all fonts/formats are supported.
