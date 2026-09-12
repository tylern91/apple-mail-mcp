mod completeness;
pub mod emlx;
pub mod extract;

pub use emlx::{AttachmentPart, EmlxAddress, EmlxFooter, ParsedMessage, parse_emlx};
pub use extract::{
    ExtractError, extract_by_content_type, html_to_text, office_xml_to_text, pdf_to_text,
};
