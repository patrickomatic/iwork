use crate::Error;
use crate::iwa::IwaArchive;
use crate::package::Package;
use crate::protobuf::ProtoMessage;

/// Message type of the Keynote theme descriptor object.
///
/// This object carries `field 1.3` = theme name (e.g. "Blueprint", "Parchment").
/// Validated across all three Keynote fixtures; field path is structurally
/// invariant (not dependent on document content).
const THEME_TYPE: u64 = 10;

/// Message type of a Keynote drawable/placeholder object.
///
/// - `field 2` (varint): placeholder kind — 1=notes, 2=title, 3=body.
///   Validated across all real slides in `with_content.key`; invariant across
///   slides with different layouts (title-only, title+body, quote, image).
/// - `field 1.4.1` (varint): object ID of the associated type-2001 text store.
///   Confirmed by correlating decoded IDs with known 2001 objects per slide.
const DRAWABLE_TYPE: u64 = 7;

/// `field 2` value in a type-7 drawable that identifies the speaker-notes placeholder.
const PLACEHOLDER_NOTES: u64 = 1;

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

/// Message type of the slide root object.
///
/// `field 10` (bytes → UTF-8): human-readable layout name, e.g. `"Blank"`,
/// `"Photo"`, `"Bullets"`. Present only in `TemplateSlide-*.iwa` archives
/// (layout masters); absent in real `Slide-*.iwa` archives. Validated across
/// all 14 layout masters in `with_content.key`; 14 distinct names, one per master.
const SLIDE_TYPE: u64 = 5;

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
    /// Sets the theme name for encoding.
    pub fn set_theme_name(&mut self, name: Option<String>) {
        self.theme_name = name;
    }

    /// Appends a slide (real or template) for encoding.
    pub fn add_slide(&mut self, slide: Slide) {
        self.slides.push(slide);
    }

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

    /// The theme name, sourced from the type-10 object in `Document.iwa` field
    /// `1.3`. Example values: `"Blueprint"`, `"21_BasicWhite"`.
    pub fn theme_name(&self) -> Option<&str> {
        self.theme_name.as_deref()
    }

    /// All slide archives (real and template), sorted by archive path.
    ///
    /// Use [`Slide::is_template`] to distinguish layout masters from real
    /// slides.
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
        .and_then(|msg| {
            msg.field(1)
                .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
        })
        .and_then(|inner| ProtoMessage::decode(&inner).ok())
        .and_then(|msg| {
            msg.field(3)
                .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
        })
        .and_then(|bytes| String::from_utf8(bytes).ok());

    Ok(name)
}

/// Decodes text from the type-7 drawable placeholder matching `kind`.
///
/// Builds a map from type-2001 object ID → formatted text, then finds the
/// type-7 drawable whose `field 2` equals `kind` and follows its
/// `field 1.4.1` cross-reference into the 2001 map.
///
/// `sep` is used to join multiple text fragments within the same 2001 object.
/// Use `" "` for single-line placeholders (title), `"\n"` for multi-paragraph
/// ones (speaker notes).
fn decode_placeholder_text(archive: &IwaArchive, kind: u64, sep: &str) -> Option<String> {
    // Build a map from 2001 object ID → formatted text.
    let text_by_id: std::collections::HashMap<u64, String> = archive
        .objects()
        .iter()
        .filter(|o| o.message_type == Some(TSWP_TEXT_TYPE))
        .filter_map(|o| {
            let id = o.identifier?;
            let raw = ProtoMessage::decode(&o.payload)
                .ok()
                .and_then(|m| {
                    m.field(3)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                })
                .and_then(|b| String::from_utf8(b).ok())?;
            let text: String = raw
                .split(|c: char| c.is_control() && c != '\n')
                .flat_map(|s| s.split('\n'))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(sep);
            if text.is_empty() {
                None
            } else {
                Some((id, text))
            }
        })
        .collect();

    // Find the drawable with the matching kind and look up its 2001 text.
    archive
        .objects()
        .iter()
        .filter(|o| o.message_type == Some(DRAWABLE_TYPE))
        .find_map(|o| {
            let msg = ProtoMessage::decode(&o.payload).ok()?;
            if msg.field(2)?.value.as_varint()? != kind {
                return None;
            }
            let ref_id = msg
                .field(1)
                .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                .and_then(|b| ProtoMessage::decode(&b).ok())
                .and_then(|m| {
                    m.field(4)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                })
                .and_then(|b| ProtoMessage::decode(&b).ok())
                .and_then(|m| m.field(1).and_then(|f| f.value.as_varint()))?;
            text_by_id.get(&ref_id).cloned()
        })
}

/// Reads the layout name from the type-5 slide root object.
///
/// Field path: `field 10` (UTF-8 bytes). Present only in `TemplateSlide-*.iwa`
/// archives; absent in real slides. Returns `None` when the field is missing or
/// not valid UTF-8.
fn decode_layout_name(archive: &IwaArchive) -> Option<String> {
    archive
        .objects()
        .iter()
        .find(|o| o.message_type == Some(SLIDE_TYPE))
        .and_then(|o| ProtoMessage::decode(&o.payload).ok())
        .and_then(|msg| {
            msg.field(10)
                .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
        })
        .and_then(|b| String::from_utf8(b).ok())
}

fn decode_slide_title(archive: &IwaArchive) -> Option<String> {
    decode_placeholder_text(archive, PLACEHOLDER_TITLE, " ")
}

fn decode_speaker_notes(archive: &IwaArchive) -> Option<String> {
    decode_placeholder_text(archive, PLACEHOLDER_NOTES, "\n")
}

/// Decodes text from TSWP text storage objects (type-2001) in a slide archive.
///
/// Each type-2001 object carries its text in `field 3` as raw UTF-8, with `\n`
/// as paragraph separators and non-whitespace control bytes (`\x04` etc.) as
/// TSWP block boundaries. We split on both, trim, and deduplicate.
fn decode_tswp_text(archive: &IwaArchive) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut fragments = Vec::new();

    for obj in archive
        .objects()
        .iter()
        .filter(|o| o.message_type == Some(TSWP_TEXT_TYPE))
    {
        let Some(raw) = ProtoMessage::decode(&obj.payload)
            .ok()
            .and_then(|msg| {
                msg.field(3)
                    .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
            })
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
                .and_then(|msg| {
                    msg.field(1)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                })
                .and_then(|inner| ProtoMessage::decode(&inner).ok())
                .and_then(|msg| {
                    msg.field(8)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                })
                .and_then(|bytes| String::from_utf8(bytes).ok())
        })
        .collect()
}

/// Structured fields decoded from a Keynote slide archive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slide {
    pub(crate) path: String,
    pub(crate) is_template: bool,
    pub(crate) layout_name: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) speaker_notes: Option<String>,
    pub(crate) text_fragments: Vec<String>,
    pub(crate) media_descriptions: Vec<String>,
}

impl Slide {
    /// Creates a new non-template slide for encoding.
    ///
    /// `text_fragments` should include the title as well as body fragments
    /// (the encoder separates them); the slide gets path `""` since it will
    /// be assigned a real path by the package writer.
    pub fn new_real(title: &str, text_fragments: Vec<String>) -> Self {
        Self {
            path: String::new(),
            is_template: false,
            layout_name: None,
            title: if title.is_empty() {
                None
            } else {
                Some(title.to_owned())
            },
            speaker_notes: None,
            text_fragments,
            media_descriptions: Vec::new(),
        }
    }

    fn from_archive(path: String, archive: &IwaArchive) -> Self {
        let title = decode_slide_title(archive);
        let speaker_notes = decode_speaker_notes(archive);
        let text_fragments = decode_tswp_text(archive);
        let media_descriptions = decode_media_descriptions(archive);
        let layout_name = decode_layout_name(archive);

        Self {
            is_template: path.starts_with(TEMPLATE_PREFIX),
            layout_name,
            media_descriptions,
            path,
            speaker_notes,
            text_fragments,
            title,
        }
    }

    /// The IWA archive path for this slide, e.g. `"Index/Slide-1.iwa"`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `true` if this is a layout master (path starts with `Index/TemplateSlide-`).
    pub fn is_template(&self) -> bool {
        self.is_template
    }

    /// The layout name, or `None` until the layout-name field path is located.
    pub fn layout_name(&self) -> Option<&str> {
        self.layout_name.as_deref()
    }

    /// The slide title, sourced from the type-7 drawable with `field 2 == 2`
    /// (title placeholder) → `field 1.4.1` object ID → type-2001 field 3.
    ///
    /// Returns `None` when the slide has no title placeholder or the referenced
    /// type-2001 object contains no text.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Speaker notes for the slide, sourced from the type-7 drawable with
    /// `field 2 == 1` (notes placeholder) → `field 1.4.1` object ID →
    /// type-2001 field 3.
    ///
    /// Paragraphs are joined with `\n`. Returns `None` when the notes area is
    /// empty.
    pub fn speaker_notes(&self) -> Option<&str> {
        self.speaker_notes.as_deref()
    }

    /// All unique text fragments from the slide, in archive order.
    ///
    /// Decoded from every type-2001 field 3 in the slide archive. Paragraphs
    /// are split on `\n`; TSWP block boundaries (non-whitespace control bytes)
    /// and U+FFFC object-replacement characters are stripped. Fragments are
    /// deduplicated.
    pub fn text_fragments(&self) -> &[String] {
        &self.text_fragments
    }

    /// Alt-text strings for all images on the slide, in archive order.
    ///
    /// Decoded from type-3005 field `1.8`.
    pub fn media_descriptions(&self) -> &[String] {
        &self.media_descriptions
    }
}
