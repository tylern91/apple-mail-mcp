//! Attachment content extraction (§4.2.4-gated): turns raw attachment bytes into searchable
//! text. Extraction is only meaningful once the completeness oracle (`completeness.rs`) confirms
//! the bytes actually reached disk — callers check that before calling into this module.
//!
//! HTML parsing goes through `html5ever` + `markup5ever_rcdom` directly rather than `scraper`:
//! `scraper` pulls in `selectors` for its CSS-selector engine (MPL-2.0, outside `deny.toml`'s
//! licence allowlist), which this module has no use for since it only needs every text node, not
//! selector matching.

use std::fmt;
use std::io::{Cursor, Read};

use html5ever::tendril::TendrilSink;
use html5ever::{ParseOpts, parse_document};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use zip::ZipArchive;

#[derive(Debug)]
pub enum ExtractError {
    Zip(String),
    Xml(String),
    Pdf(String),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractError::Zip(reason) => write!(f, "zip archive error: {reason}"),
            ExtractError::Xml(reason) => write!(f, "xml parse error: {reason}"),
            ExtractError::Pdf(reason) => write!(f, "pdf parse error: {reason}"),
        }
    }
}

impl std::error::Error for ExtractError {}

/// Strips markup from an HTML body, keeping only visible text nodes, whitespace-collapsed.
/// `<script>`/`<style>` contents are never visible text, so they are skipped.
pub fn html_to_text(html: &str) -> String {
    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("in-memory reads are infallible");

    let mut text = String::new();
    collect_text(&dom.document, &mut text);
    collapse_whitespace(&text)
}

fn collect_text(node: &Handle, text: &mut String) {
    match &node.data {
        NodeData::Text { contents } => {
            text.push_str(&contents.borrow());
            text.push(' ');
        }
        NodeData::Element { name, .. } => {
            if matches!(&*name.local, "script" | "style") {
                return;
            }
        }
        _ => {}
    }

    for child in node.children.borrow().iter() {
        collect_text(child, text);
    }
}

/// Extracts text from every page of a PDF attachment, in page order.
pub fn pdf_to_text(bytes: &[u8]) -> Result<String, ExtractError> {
    let document =
        lopdf::Document::load_mem(bytes).map_err(|err| ExtractError::Pdf(err.to_string()))?;
    let page_numbers: Vec<u32> = document.get_pages().keys().copied().collect();
    document
        .extract_text(&page_numbers)
        .map_err(|err| ExtractError::Pdf(err.to_string()))
}

/// Extracts text from an Office Open XML container (DOCX/PPTX/XLSX) by walking every `.xml`
/// entry in the zip and collecting the contents of `<w:t>`/`<a:t>`/shared-string `<t>` elements —
/// one heuristic covering all three formats rather than three near-duplicate per-format parsers.
pub fn office_xml_to_text(bytes: &[u8]) -> Result<String, ExtractError> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|err| ExtractError::Zip(err.to_string()))?;

    let mut chunks = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|err| ExtractError::Zip(err.to_string()))?;
        if !entry.name().ends_with(".xml") {
            continue;
        }
        let mut contents = String::new();
        entry
            .read_to_string(&mut contents)
            .map_err(|err| ExtractError::Zip(err.to_string()))?;
        chunks.push(extract_t_elements(&contents)?);
    }

    Ok(collapse_whitespace(&chunks.join(" ")))
}

/// Collects the text content of every `<t>`-local-named element (covers `<w:t>` in DOCX,
/// `<a:t>` in PPTX, and shared-string `<t>` in XLSX) in document order.
fn extract_t_elements(xml: &str) -> Result<String, ExtractError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut text = String::new();
    let mut in_text_element = false;

    loop {
        match reader
            .read_event()
            .map_err(|err| ExtractError::Xml(err.to_string()))?
        {
            Event::Start(tag) if tag.local_name().as_ref() == b"t" => in_text_element = true,
            Event::End(tag) if tag.local_name().as_ref() == b"t" => in_text_element = false,
            Event::Text(bytes) if in_text_element => {
                let decoded = bytes
                    .decode()
                    .map_err(|err| ExtractError::Xml(err.to_string()))?;
                let unescaped =
                    unescape(&decoded).map_err(|err| ExtractError::Xml(err.to_string()))?;
                text.push_str(&unescaped);
                text.push(' ');
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(text)
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Dispatches `bytes` to the extractor matching `(type_, subtype)`, or `None` if this module has
/// no extractor for that MIME type — the caller treats an unrecognized type the same as an
/// extraction failure (`AttachmentState::Unextractable`), not a hard error.
pub fn extract_by_content_type(
    type_: &str,
    subtype: Option<&str>,
    bytes: &[u8],
) -> Option<Result<String, ExtractError>> {
    let subtype = subtype.map(str::to_ascii_lowercase);
    match (type_.to_ascii_lowercase().as_str(), subtype.as_deref()) {
        ("text", Some("html")) => Some(Ok(html_to_text(&String::from_utf8_lossy(bytes)))),
        ("application", Some(sub)) if sub.contains("pdf") => Some(pdf_to_text(bytes)),
        ("application", Some(sub)) if is_office_xml_subtype(sub) => Some(office_xml_to_text(bytes)),
        _ => None,
    }
}

fn is_office_xml_subtype(subtype: &str) -> bool {
    subtype.contains("wordprocessingml")
        || subtype.contains("presentationml")
        || subtype.contains("spreadsheetml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn html_to_text_strips_tags_and_collapses_whitespace() {
        let html = "<html><body><p>Hello   <b>world</b></p>\n<p>Second line</p></body></html>";
        assert_eq!(html_to_text(html), "Hello world Second line");
    }

    #[test]
    fn html_to_text_skips_script_and_style_content() {
        let html = "<html><head><style>.x{color:red}</style></head><body><script>alert(1)</script><p>Visible</p></body></html>";
        assert_eq!(html_to_text(html), "Visible");
    }

    #[test]
    fn pdf_to_text_reads_back_a_freshly_built_document() {
        // lopdf's own writer is the only reliably round-trippable source in this crate's
        // version, so the fixture is built here rather than committed as a binary blob — the
        // exact lesson learned from the .emlx CRLF-corruption bug: don't commit binary-shaped
        // fixtures when a faithful in-memory construction is available instead.
        use lopdf::{Document, Object, Stream, dictionary};

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let content = lopdf::content::Content {
            operations: vec![
                lopdf::content::Operation::new("BT", vec![]),
                lopdf::content::Operation::new("Tf", vec!["F1".into(), 24.into()]),
                lopdf::content::Operation::new("Td", vec![100.into(), 700.into()]),
                lopdf::content::Operation::new("Tj", vec![Object::string_literal("fixture text")]),
                lopdf::content::Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
                "Resources" => resources_id,
            }),
        );

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let text = pdf_to_text(&bytes).unwrap();
        assert!(text.contains("fixture text"), "got: {text:?}");
    }

    #[test]
    fn office_xml_to_text_reads_wordprocessing_and_presentation_and_spreadsheet_runs() {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
            let options = SimpleFileOptions::default();

            zip.start_file("word/document.xml", options).unwrap();
            zip.write_all(
                br#"<w:document xmlns:w="ns"><w:body><w:p><w:r><w:t>Docx paragraph</w:t></w:r></w:p></w:body></w:document>"#,
            )
            .unwrap();

            zip.start_file("ppt/slides/slide1.xml", options).unwrap();
            zip.write_all(br#"<p:sld xmlns:a="ns"><a:t>Slide title</a:t></p:sld>"#)
                .unwrap();

            zip.start_file("xl/sharedStrings.xml", options).unwrap();
            zip.write_all(br#"<sst><si><t>Cell value</t></si></sst>"#)
                .unwrap();

            zip.finish().unwrap();
        }

        let text = office_xml_to_text(&buf).unwrap();
        assert!(text.contains("Docx paragraph"), "got: {text:?}");
        assert!(text.contains("Slide title"), "got: {text:?}");
        assert!(text.contains("Cell value"), "got: {text:?}");
    }

    #[test]
    fn office_xml_to_text_rejects_a_non_zip_blob() {
        let err = office_xml_to_text(b"not a zip file").unwrap_err();
        assert!(matches!(err, ExtractError::Zip(_)));
    }

    #[test]
    fn pdf_to_text_rejects_a_non_pdf_blob() {
        let err = pdf_to_text(b"not a pdf file").unwrap_err();
        assert!(matches!(err, ExtractError::Pdf(_)));
    }

    #[test]
    fn extract_by_content_type_dispatches_html_to_the_html_extractor() {
        let result = extract_by_content_type("text", Some("html"), b"<p>Hi</p>");
        assert_eq!(result.unwrap().unwrap(), "Hi");
    }

    #[test]
    fn extract_by_content_type_has_no_extractor_for_an_unknown_type() {
        assert!(extract_by_content_type("image", Some("png"), b"\x89PNG").is_none());
    }
}
