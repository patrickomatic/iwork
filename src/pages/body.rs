use crate::iwa::IwaArchive;
use crate::iwa_text::extract_utf8_fields;
use crate::protobuf::ProtoMessage;
use crate::{Error, Package};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";

/// Message type of the Pages word-processor body object.
///
/// This object carries `field 1.3` = template name (e.g. `04B_Term_Paper`).
/// Validated across both Pages fixtures; field path is structurally invariant.
const WP_BODY_TYPE: u64 = 10001;

/// Message type of a media/image object.
///
/// field 1 (bytes) → field 8 (bytes → UTF-8): image alt-text.
/// Identical to the Keynote type-3005 encoding; validated in term_paper and
/// modern_novel (2 type-3005 objects each in Document.iwa).
const MEDIA_TYPE: u64 = 3005;

/// UTF-8 string fields decoded from a Pages document archive.
///
/// This is intentionally structural but narrow: it walks the decoded IWA
/// protobuf fields and returns valid UTF-8 length-delimited values. It does not
/// classify titles, headings, paragraphs, or text runs until those Pages object
/// fields are decoded explicitly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Body {
    template_name: Option<String>,
    title: Option<String>,
    headings: Vec<String>,
    text_fragments: Vec<String>,
    media_descriptions: Vec<String>,
}

impl Body {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let bytes = package.entry_bytes(DOCUMENT_ENTRY)?;
        let archive = IwaArchive::decode(bytes)?;
        let template_name = decode_template_name(&archive);
        let text_fragments = extract_utf8_fields(&archive);
        let media_descriptions = decode_media_descriptions(&archive);

        Ok(Self {
            template_name,
            title: None,
            headings: Vec::new(),
            text_fragments,
            media_descriptions,
        })
    }

    /// The iWork template identifier used to create this document, if present.
    pub fn template_name(&self) -> Option<&str> {
        self.template_name.as_deref()
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

    pub fn media_descriptions(&self) -> &[String] {
        &self.media_descriptions
    }
}

/// Reads the template name from the type-10001 object in `Document.iwa`.
///
/// Field path: `field 1` (nested) → `field 3` (UTF-8 string).
fn decode_template_name(archive: &IwaArchive) -> Option<String> {
    archive
        .objects()
        .into_iter()
        .find(|obj| obj.message_type == Some(WP_BODY_TYPE))
        .and_then(|obj| ProtoMessage::decode(&obj.payload).ok())
        .and_then(|msg| msg.field(1).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
        .and_then(|inner| ProtoMessage::decode(&inner).ok())
        .and_then(|msg| msg.field(3).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

/// Reads alt-text from all type-3005 media objects in `Document.iwa`.
///
/// Field path: `field 1` (nested) → `field 8` (UTF-8 string).
fn decode_media_descriptions(archive: &IwaArchive) -> Vec<String> {
    archive
        .objects()
        .iter()
        .filter(|obj| obj.message_type == Some(MEDIA_TYPE))
        .filter_map(|obj| {
            ProtoMessage::decode(&obj.payload)
                .ok()
                .and_then(|msg| msg.field(1).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
                .and_then(|inner| ProtoMessage::decode(&inner).ok())
                .and_then(|msg| msg.field(8).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
                .and_then(|bytes| String::from_utf8(bytes).ok())
        })
        .collect()
}
