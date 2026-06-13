use crate::iwa::IwaArchive;
use crate::protobuf::ProtoMessage;
use crate::{Error, Package};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";

/// Message type of the Pages word-processor body object.
///
/// This object carries `field 1.3` = template name (e.g. `04B_Term_Paper`).
/// Validated across both Pages fixtures; field path is structurally invariant.
const WP_BODY_TYPE: u64 = 10001;

/// Message type of a TSWP text storage object.
///
/// field 3 (bytes): the raw UTF-8 document text, where `\n` separates paragraphs
/// and TSWP block markers (`\x04`, etc.) delimit sections. Validated in
/// eternal_sunshine.pages (9165-byte field 3 beginning "Author Name\n\nEternal Shine").
/// Blueprint/parchment Keynote 2001 objects have no field 3 (empty text areas);
/// field 3 is only present when the text area has actual content.
const TSWP_TEXT_TYPE: u64 = 2001;

/// Message type of a media/image object.
///
/// field 1 (bytes) → field 8 (bytes → UTF-8): image alt-text.
/// Identical to the Keynote type-3005 encoding; validated in term_paper and
/// modern_novel (2 type-3005 objects each in Document.iwa).
const MEDIA_TYPE: u64 = 3005;

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
        let text_fragments = decode_tswp_text(&archive);
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

/// Decodes all text from TSWP text storage objects (type-2001) in the archive.
///
/// Each type-2001 object carries its text in `field 3` as raw UTF-8. Paragraphs
/// are separated by `\n`; TSWP block boundaries use non-whitespace control bytes
/// (`\x04` and similar). We split on any non-whitespace control char and on `\n`,
/// then trim and discard empty fragments.
fn decode_tswp_text(archive: &IwaArchive) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut fragments = Vec::new();

    for obj in archive.objects().iter().filter(|o| o.message_type == Some(TSWP_TEXT_TYPE)) {
        let Some(raw) = ProtoMessage::decode(&obj.payload)
            .ok()
            .and_then(|msg| msg.field(3).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            continue;
        };

        for fragment in raw
            .split(|c: char| c.is_control() && c != '\n')
            .flat_map(|seg| seg.split('\n'))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if seen.insert(fragment.to_owned()) {
                fragments.push(fragment.to_owned());
            }
        }
    }

    fragments
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
