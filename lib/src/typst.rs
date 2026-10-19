use crate::core::Element::{Header, Hyperlink, Image, List, Paragraph, Table, Text};

use crate::core::{Document, Element, ListItem, TableHeader, TableRow, TransformerTrait};
use anyhow;
use bytes::Bytes;
use comemo::Prehashed;
use log::warn;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use time::{OffsetDateTime, UtcOffset};

use typst::{
    diag::{FileError, FileResult},
    foundations::Datetime,
    syntax::{FileId, Source},
    text::{Font, FontBook},
    Library, World,
};

type TypstString = String;

pub struct ShivaWorld {
    fonts: Vec<Font>,
    book: Prehashed<FontBook>,
    library: Prehashed<Library>,
    source: Source,
    img_map: HashMap<String, typst::foundations::Bytes>,
}

impl ShivaWorld {
    /// Construct a PDF world with no implicit network or filesystem writes.
    ///
    /// Fonts must already be provisioned. The host may select an existing
    /// directory via SHIVA_FONT_DIR; otherwise the local ./fonts directory
    /// is used. In Akula this directory must be explicitly authorized.
    pub fn new(
        source: String,
        img_map: HashMap<String, typst::foundations::Bytes>,
    ) -> anyhow::Result<Self> {
        let font_dir = std::env::var_os("SHIVA_FONT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("fonts"));
        Self::with_font_dir(source, img_map, &font_dir)
    }

    /// Read only caller-provisioned font files from a bounded local directory.
    /// Invalid fonts and unexpected file types are skipped; the absence of
    /// usable fonts is a normal error rather than a panic or download.
    pub fn with_font_dir(
        source: String,
        img_map: HashMap<String, typst::foundations::Bytes>,
        font_dir: &Path,
    ) -> anyhow::Result<Self> {
        const MAX_FILES: usize = 256;
        const MAX_FONT_SIZE: u64 = 64 * 1024 * 1024;
        const MAX_TOTAL_SIZE: u64 = 256 * 1024 * 1024;
        const MAX_FACES_PER_FILE: u32 = 64;

        let mut paths = Vec::new();
        for entry in std::fs::read_dir(font_dir)? {
            if paths.len() >= MAX_FILES {
                anyhow::bail!("font directory contains too many entries");
            }
            paths.push(entry?.path());
        }
        paths.sort();

        let mut fonts = Vec::new();
        let mut total_size = 0u64;
        for path in paths {
            // Do not follow symlinked fonts. The requested font directory
            // is host-controlled, not provided by a document being parsed.
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() {
                continue;
            }
            let valid_extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    ext.eq_ignore_ascii_case("ttf")
                        || ext.eq_ignore_ascii_case("otf")
                        || ext.eq_ignore_ascii_case("ttc")
                })
                .unwrap_or(false);
            if !valid_extension {
                continue;
            }
            if metadata.len() > MAX_FONT_SIZE {
                anyhow::bail!("font is too large");
            }
            total_size = total_size
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow::anyhow!("font size overflow"))?;
            if total_size > MAX_TOTAL_SIZE {
                anyhow::bail!("font directory exceeds byte budget");
            }

            let buffer = typst::foundations::Bytes::from(std::fs::read(&path)?);
            let face_count = ttf_parser::fonts_in_collection(&buffer).unwrap_or(1);
            if face_count > MAX_FACES_PER_FILE {
                anyhow::bail!("font collection exceeds face budget");
            }
            for face in 0..face_count {
                if let Some(font) = Font::new(buffer.clone(), face) {
                    fonts.push(font);
                }
            }
        }
        if fonts.is_empty() {
            anyhow::bail!("font directory has no usable fonts; configure SHIVA_FONT_DIR");
        }

        let source = Source::detached(source);
        Ok(Self {
            book: Prehashed::new(FontBook::from_fonts(&fonts)),
            fonts,
            library: Prehashed::new(Library::default()),
            source,
            img_map,
        })
    }
}

#[cfg(test)]
mod offline_font_tests {
    use super::*;

    #[test]
    fn missing_font_directory_does_not_create_or_download() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test")
            .join("missing-offline-fonts");
        assert!(!folder.exists(), "fixture must be absent");
        let world = ShivaWorld::with_font_dir("Hello".to_string(), HashMap::new(), &folder);
        assert!(world.is_err(), "missing fonts must be a normal error");
        assert!(!folder.exists(), "Shiva must not create the directory");
    }

    #[test]
    fn checked_in_fonts_load_without_network() -> anyhow::Result<()> {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts");
        let world = ShivaWorld::with_font_dir("Hello".to_string(), HashMap::new(), &folder)?;
        assert!(!world.fonts.is_empty());
        Ok(())
    }

    #[test]
    fn corrupt_font_is_not_a_panic() -> anyhow::Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let folder = std::env::temp_dir()
            .join(format!("shiva-invalid-font-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&folder)?;
        std::fs::write(folder.join("invalid.ttf"), b"not a font")?;
        let result = ShivaWorld::with_font_dir("Hello".to_string(), HashMap::new(), &folder);
        assert!(result.is_err());
        std::fs::remove_file(folder.join("invalid.ttf"))?;
        std::fs::remove_dir(folder)?;
        Ok(())
    }
}

impl World for ShivaWorld {
    fn book(&self) -> &Prehashed<FontBook> {
        &self.book
    }

    fn library(&self) -> &Prehashed<Library> {
        &self.library
    }

    fn main(&self) -> Source {
        self.source.clone()
    }

    fn source(&self, _id: FileId) -> FileResult<Source> {
        Ok(self.source.clone())
    }

    fn font(&self, id: usize) -> Option<Font> {
        self.fonts.get(id).cloned()
    }

    // need to think how to implement path and file extraction
    fn file(&self, id: FileId) -> Result<typst::foundations::Bytes, FileError> {
        let path = id.vpath();

        let key = path.as_rootless_path().to_str().unwrap();
        let img = self.img_map.get(key).unwrap();

        Ok(img.clone())
    }

    fn today(&self, offset: Option<i64>) -> Option<Datetime> {
        // We are in UTC.
        let offset = offset.unwrap_or(0);
        let offset = UtcOffset::from_hms(offset.try_into().ok()?, 0, 0).ok()?;
        let time = OffsetDateTime::now_utc().checked_to_offset(offset)?;
        Some(Datetime::Date(time.date()))
    }
}

pub struct Transformer;

impl TransformerTrait for Transformer {
    #[allow(unused)]
    fn parse(document: &bytes::Bytes) -> anyhow::Result<Document> {
        todo!()
    }

    fn generate(document: &Document) -> anyhow::Result<bytes::Bytes> {
        let (text, _) = generate_document(document)?;
        let bytes = Bytes::from(text);
        Ok(bytes)
    }
}

/// Converts Document into a typst::model::Document
pub fn generate_document(
    document: &Document,
) -> anyhow::Result<(TypstString, HashMap<String, typst::foundations::Bytes>)> {
    // Array of methods to process Document object into a typst string repr
    fn process_header(source: &mut TypstString, level: usize, text: &str) -> anyhow::Result<()> {
        let header_depth = "=".repeat(level);
        let header_text = format!("{header_depth} {text}");
        source.push_str(&header_text);
        source.push('\n');

        Ok(())
    }

    fn process_text(
        source: &mut TypstString,
        _size: u8,
        text: &str,
        is_bold: bool,
    ) -> anyhow::Result<()> {
        if is_bold {
            let bold_text = format!("*{text}*");
            source.push_str(&bold_text);
        } else {
            source.push_str(text);
        }

        Ok(())
    }

    fn process_link(source: &mut TypstString, url: &str) -> anyhow::Result<()> {
        let link = format!("#link(\"{url}\")");

        source.push_str(&link);

        Ok(())
    }

    fn process_table(
        source: &mut TypstString,
        headers: &Vec<TableHeader>,
        rows: &Vec<TableRow>,
    ) -> anyhow::Result<()> {
        let mut headers_text = TypstString::new();

        for header in headers {
            match &header.element {
                Text { text, size } => {
                    headers_text.push('[');
                    process_text(&mut headers_text, *size, text, true)?;
                    headers_text.push(']');
                    headers_text.push(',');
                }
                _ => {
                    warn!(
                        "Should implement element for processing in inside table header - {:?}",
                        header.element
                    );
                }
            }
        }

        let mut cells_text = TypstString::new();

        for row in rows {
            for cell in &row.cells {
                match &cell.element {
                    Text { text, size } => {
                        cells_text.push('[');
                        process_text(&mut cells_text, *size, text, false)?;
                        cells_text.push(']');
                        cells_text.push(',');
                    }
                    _ => {
                        warn!(
                            "Should implement element for processing in inside cell - {:?}",
                            cell.element
                        );
                    }
                }
            }

            cells_text.push('\n');
        }

        let columns = headers.len();
        let table_text = format!(
            r#"
        #table(
            columns:{columns},
            {headers_text}
            {cells_text}
        )
        "#
        );

        source.push_str(&table_text);
        Ok(())
    }

    fn process_list(
        source: &mut TypstString,
        img_map: &mut HashMap<String, typst::foundations::Bytes>,
        list: &Vec<ListItem>,
        numbered: bool,
        depth: usize,
    ) -> anyhow::Result<()> {
        source.push_str(&" ".repeat(depth));
        for el in list {
            if let List { elements, numbered } = &el.element {
                process_list(source, img_map, elements, *numbered, depth + 1)?;
            } else {
                if numbered {
                    source.push_str("+ ")
                } else {
                    source.push_str("- ")
                };

                process_element(source, img_map, &el.element)?;
            }
        }

        Ok(())
    }

    fn process_image(
        source: &mut TypstString,
        bytes: &Bytes,
        title: &str,
        alt: &str,
        image_type: &str,
    ) -> anyhow::Result<()> {
        if !bytes.is_empty() {
            let image_text = format!(
                "
            #image(\"{title}{image_type}\", alt: \"{alt}\")
            "
            );
            source.push_str(&image_text);
        }
        // need to think how to implement using raw bytes
        Ok(())
    }

    fn process_element(
        source: &mut TypstString,
        img_map: &mut HashMap<String, typst::foundations::Bytes>,
        element: &Element,
    ) -> anyhow::Result<()> {
        match element {
            Header { level, text } => process_header(source, *level as usize, text),
            Paragraph { elements } => {
                for paragraph_element in elements {
                    process_element(source, img_map, paragraph_element)?;
                }

                Ok(())
            }
            Text { text, size } => {
                process_text(source, *size, text, false)?;
                source.push('\n');

                Ok(())
            }
            List { elements, numbered } => {
                process_list(source, img_map, elements, *numbered, 0)?;
                Ok(())
            }
            Hyperlink {
                url,
                title: _,
                alt: _,
                size: _,
            } => {
                process_link(source, url)?;
                source.push('\n');

                Ok(())
            }
            Table { headers, rows } => {
                process_table(source, headers, rows)?;
                Ok(())
            }
            Image(image) => {
                let key = format!("{}{}", image.title(), image.image_type());
                img_map.insert(key, typst::foundations::Bytes::from(image.bytes().to_vec()));
                process_image(
                    source,
                    image.bytes(),
                    image.title(),
                    image.alt(),
                    &image.image_type().to_string(),
                )?;
                source.push('\n');
                Ok(())
            } // _ => {
              //     warn!("Should implement element - {:?}", element);
              //     Ok(())
              // }
        }
    }

    // String to build off of
    let mut source = TypstString::new();
    // Mapping of connections between elements
    let mut img_map: HashMap<String, typst::foundations::Bytes> = HashMap::new();

    // Converting both headers and footers into a string repr of them in Typst
    let mut header_text = String::new();
    document.get_page_header().iter().for_each(|el| match el {
        Text { text, size: _ } => {
            header_text.push_str(text);
        }
        _ => {}
    });
    let mut footer_text = String::new();
    document.get_page_footer().iter().for_each(|el| match el {
        Text { text, size: _ } => {
            footer_text.push_str(text);
        }
        _ => {}
    });
    let footer_header_text = format!(
        "#set page(
        header: \"{header_text}\",
        footer: \"{footer_text}\",
    )\n"
    );

    // Converting Document repr to one of typst string
    source.push_str(&footer_header_text);
    for element in &document.get_all_elements() {
        process_element(&mut source, &mut img_map, element)?;
    }

    Ok((source, img_map))
}

#[cfg(test)]
mod test {
    use crate::core::{disk_image_loader, TransformerWithImageLoaderSaverTrait};
    use crate::markdown;
    use bytes::Bytes;

    use super::*;
    #[test]
    fn test_generate() -> anyhow::Result<()> {
        let document = std::fs::read("test/data/document.md")?;
        let documents_bytes = Bytes::from(document);
        let parsed = markdown::Transformer::parse_with_loader(
            &documents_bytes,
            disk_image_loader("test/data"),
        )?;
        let generated_result = crate::typst::Transformer::generate(&parsed)?;
        std::fs::write("test/data/document_from_md.typ", generated_result)?;

        Ok(())
    }

    #[test]
    fn test_generate_from_xml() -> anyhow::Result<()> {
        let document = std::fs::read("test/data/document.xml")?;
        let documents_bytes = Bytes::from(document);
        let parsed = crate::xml::Transformer::parse(&documents_bytes)?;
        let generated_result = crate::typst::Transformer::generate(&parsed)?;
        std::fs::write("test/data/document_from_xml.typ", generated_result)?;

        Ok(())
    }
}
