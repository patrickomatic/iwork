use crate::iwa::IwaArchive;
use crate::protobuf::ProtoMessage;
use crate::{Error, Package};

/// Message type of the Keynote theme descriptor object.
///
/// This object carries `field 1.3` = theme name (e.g. "Blueprint", "Parchment").
/// Validated across all three Keynote fixtures; field path is structurally
/// invariant (not dependent on document content).
const THEME_TYPE: u64 = 10;

/// Message type of a Keynote drawable/placeholder object.
///
/// - `field 2` (varint): placeholder kind — 1=notes, 2=title, 3=body.
///   Validated across all real slides in with_content.key; invariant across
///   slides with different layouts (title-only, title+body, quote, image).
/// - `field 1.4.1` (varint): object ID of the associated type-2001 text store.
///   Confirmed by correlating decoded IDs with known 2001 objects per slide.
const DRAWABLE_TYPE: u64 = 7;

/// `field 2` value in a type-7 drawable that identifies the title placeholder.
const PLACEHOLDER_TITLE: u64 = 2;

/// Message type of a TSWP text storage object.
///
/// field 3 (bytes): raw UTF-8 slide text; `\n` = paragraph break, TSWP block
/// markers (`\x04`, etc.) delimit sections. Current Keynote fixtures have empty
/// 2001 objects (no field 3); this decoder becomes active once a fixture has
/// real slide content. Same encoding as Pages type-2001 field 3.
const TSWP_TEXT_TYPE: u64 = 2001;

/// Message type of the Keynote media/image object.
///
/// field 1 (bytes) → field 8 (bytes → UTF-8): image alt-text.
/// Validated across all Keynote fixtures that include images.
const MEDIA_TYPE: u64 = 3005;

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";

/// Prefix that identifies a real (non-template) slide archive.
const SLIDE_PREFIX: &str = "Index/Slide-";
/// Prefix that identifies a template slide archive (layout masters).
const TEMPLATE_PREFIX: &str = "Index/TemplateSlide-";
const IWA_EXT: &str = ".iwa";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presentation {
    theme_name: Option<String>,
    slides: Vec<Slide>,
}

impl Presentation {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let theme_name = decode_theme_name(package)?;

        let mut slides = package
            .entries()
            .iter()
            .filter(|entry| {
                let p = &entry.path;
                (p.starts_with(SLIDE_PREFIX) || p.starts_with(TEMPLATE_PREFIX))
                    && p.ends_with(IWA_EXT)
            })
            .map(|entry| {
                let archive = IwaArchive::decode(package.entry_bytes(&entry.path)?)?;
                Ok(Slide::from_archive(entry.path.clone(), &archive))
            })
            .collect::<Result<Vec<_>, Error>>()?;

        slides.sort_by(|left, right| left.path.cmp(&right.path));

        Ok(Self { theme_name, slides })
    }

    pub fn theme_name(&self) -> Option<&str> {
        self.theme_name.as_deref()
    }

    pub fn slides(&self) -> &[Slide] {
        &self.slides
    }
}

/// Reads the theme name from the type-10 object in `Document.iwa`.
///
/// Field path: `field 1` (nested) → `field 3` (UTF-8 string).
fn decode_theme_name(package: &Package) -> Result<Option<String>, Error> {
    let bytes = package.entry_bytes(DOCUMENT_ENTRY)?;
    let archive = IwaArchive::decode(bytes)?;

    let name = archive
        .objects()
        .into_iter()
        .find(|obj| obj.message_type == Some(THEME_TYPE))
        .and_then(|obj| ProtoMessage::decode(&obj.payload).ok())
        .and_then(|msg| msg.field(1).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
        .and_then(|inner| ProtoMessage::decode(&inner).ok())
        .and_then(|msg| msg.field(3).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
        .and_then(|bytes| String::from_utf8(bytes).ok());

    Ok(name)
}

/// Decodes the slide title from type-7 drawable objects.
///
/// Finds the type-7 with `field 2 == PLACEHOLDER_TITLE (2)`, follows its
/// `field 1.4.1` reference to the associated type-2001, and reads field 3.
fn decode_slide_title(archive: &IwaArchive) -> Option<String> {
    // Build a map from 2001 object ID → text so we can look up by reference.
    let text_by_id: std::collections::HashMap<u64, String> = archive
        .objects()
        .iter()
        .filter(|o| o.message_type == Some(TSWP_TEXT_TYPE))
        .filter_map(|o| {
            let id = o.identifier?;
            let raw = ProtoMessage::decode(&o.payload)
                .ok()
                .and_then(|m| m.field(3).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
                .and_then(|b| String::from_utf8(b).ok())?;
            let text: String = raw
                .split(|c: char| c.is_control() && c != '\n')
                .flat_map(|s| s.split('\n'))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if text.is_empty() { None } else { Some((id, text)) }
        })
        .collect();

    // Find the title drawable and look up its 2001 text.
    archive
        .objects()
        .iter()
        .filter(|o| o.message_type == Some(DRAWABLE_TYPE))
        .find_map(|o| {
            let msg = ProtoMessage::decode(&o.payload).ok()?;
            let kind = msg.field(2)?.value.as_varint()?;
            if kind != PLACEHOLDER_TITLE {
                return None;
            }
            let ref_id = msg
                .field(1).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                .and_then(|b| ProtoMessage::decode(&b).ok())
                .and_then(|m| m.field(4).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
                .and_then(|b| ProtoMessage::decode(&b).ok())
                .and_then(|m| m.field(1).and_then(|f| f.value.as_varint()))?;
            text_by_id.get(&ref_id).cloned()
        })
}

/// Decodes text from TSWP text storage objects (type-2001) in a slide archive.
///
/// Each type-2001 object carries its text in `field 3` as raw UTF-8, with `\n`
/// as paragraph separators and non-whitespace control bytes (`\x04` etc.) as
/// TSWP block boundaries. We split on both, trim, and deduplicate.
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
            .filter(|s| !s.is_empty() && !s.contains('\u{FFFC}'))
        {
            if seen.insert(fragment.to_owned()) {
                fragments.push(fragment.to_owned());
            }
        }
    }

    fragments
}

/// Reads alt-text from all type-3005 media objects in the archive.
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

/// Structured fields decoded from a Keynote slide archive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slide {
    path: String,
    is_template: bool,
    layout_name: Option<String>,
    title: Option<String>,
    text_fragments: Vec<String>,
    media_descriptions: Vec<String>,
}

impl Slide {
    fn from_archive(path: String, archive: &IwaArchive) -> Self {
        let title = decode_slide_title(archive);
        let text_fragments = decode_tswp_text(archive);
        let media_descriptions = decode_media_descriptions(archive);

        Self {
            is_template: path.starts_with(TEMPLATE_PREFIX),
            layout_name: None,
            media_descriptions,
            path,
            text_fragments,
            title,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn is_template(&self) -> bool {
        self.is_template
    }

    pub fn layout_name(&self) -> Option<&str> {
        self.layout_name.as_deref()
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn text_fragments(&self) -> &[String] {
        &self.text_fragments
    }

    pub fn media_descriptions(&self) -> &[String] {
        &self.media_descriptions
    }
}
