use crate::iwa::IwaArchive;
use crate::iwa_text::extract_utf8_fields;
use crate::{Error, Package};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";

/// UTF-8 string fields decoded from a Pages document archive.
///
/// This is intentionally structural but narrow: it walks the decoded IWA
/// protobuf fields and returns valid UTF-8 length-delimited values. It does not
/// classify titles, headings, paragraphs, or text runs until those Pages object
/// fields are decoded explicitly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticDocument {
    title: Option<String>,
    headings: Vec<String>,
    text_fragments: Vec<String>,
}

impl SemanticDocument {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let bytes = package.entry_bytes(DOCUMENT_ENTRY)?;
        let archive = IwaArchive::decode(bytes)?;
        let text_fragments = extract_utf8_fields(&archive);

        Ok(Self {
            title: None,
            headings: Vec::new(),
            text_fragments,
        })
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn headings(&self) -> &[String] {
        &self.headings
    }

    pub fn text_fragments(&self) -> &[String] {
        &self.text_fragments
    }
}
