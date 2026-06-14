use std::collections::HashMap;
use crate::iwa::IwaArchive;
use crate::protobuf::ProtoMessage;
use crate::package::Package;
use crate::Error;

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";
const STYLESHEET_ENTRY: &str = "Index/DocumentStylesheet.iwa";

/// Message type of the Pages word-processor body object.
///
/// This object carries `field 1.3` = template name (e.g. `04B_Term_Paper`).
/// Validated across both Pages fixtures; field path is structurally invariant.
const WP_BODY_TYPE: u64 = 10001;

/// Message type of the `DocumentStylesheet` object.
///
/// field 2 (repeated): named-style entries — each has field 2.1 (UTF-8 style
/// key, e.g. `"text-0-paragraphstyle-Title"`) and field 2.2.1 (uint64 object
/// ID of the style object).  Validated across all Pages fixtures.
const STYLESHEET_TYPE: u64 = 401;

/// Message type of a TSWP text storage object.
///
/// field 3 (bytes): the raw UTF-8 document text, where `\n` separates paragraphs
/// and TSWP block markers (`\x04`, etc.) delimit sections. Validated in
/// `eternal_sunshine.pages` (9165-byte field 3 beginning "Author Name\n\nEternal Shine").
/// Blueprint/parchment Keynote 2001 objects have no field 3 (empty text areas);
/// field 3 is only present when the text area has actual content.
///
/// field 5 (bytes): repeated paragraph-style runs.  Each run entry has:
///   - field 5.1.1 (uint64): byte offset in field 3 where this style begins.
///   - field 5.1.2.1 (uint64): style object ID referenced from the 401
///     stylesheet's field 2 named-style table.
///
/// Validated by correlating corpus-inferred field-5 IDs (e.g. 1165693,
/// 1165682) with the corresponding `dump_stylesheet` output for
/// `eternal_sunshine.pages`.
const TSWP_TEXT_TYPE: u64 = 2001;

/// Message type of a media/image object.
///
/// field 1 (bytes) → field 8 (bytes → UTF-8): image alt-text.
/// Identical to the Keynote type-3005 encoding; validated in `term_paper` and
/// `modern_novel` (2 type-3005 objects each in `Document.iwa`).
const MEDIA_TYPE: u64 = 3005;

/// Substring that identifies a paragraph-level style key in the 401 table.
///
/// Keys follow the pattern `"text-{n}-paragraphstyle-{Name}"` (e.g.
/// `"text-0-paragraphstyle-Title"`). Splitting on this separator and taking the
/// trailing part gives the human-readable style name.
const PARA_STYLE_SEP: &str = "-paragraphstyle-";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Body {
    pub(crate) template_name: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) headings: Vec<String>,
    pub(crate) text_fragments: Vec<String>,
    pub(crate) media_descriptions: Vec<String>,
}

impl Body {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let bytes = package.entry_bytes(DOCUMENT_ENTRY)?;
        let archive = IwaArchive::decode(bytes)?;
        let template_name = decode_template_name(&archive);

        let style_map = decode_paragraph_style_map(package)?;
        let (title, headings, text_fragments) = decode_classified_text(&archive, &style_map);
        let media_descriptions = decode_media_descriptions(&archive);

        Ok(Self {
            template_name,
            title,
            headings,
            text_fragments,
            media_descriptions,
        })
    }

    /// The iWork template identifier used to create this document, if present.
    ///
    /// Sourced from type-10001 field `1.3`. Example values: `"04B_Term_Paper"`,
    /// `"11B_Novel_Modern"`. Returns `None` for documents not based on a named
    /// template.
    pub fn template_name(&self) -> Option<&str> {
        self.template_name.as_deref()
    }

    /// The document title, sourced from the first paragraph whose paragraph
    /// style key in the 401 stylesheet contains `"paragraphstyle-Title"`.
    ///
    /// Returns `None` when no title-styled paragraph is present or non-empty.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Heading strings from the document body, in document order.
    ///
    /// Includes paragraphs whose paragraph style name contains "Heading",
    /// "Chapter", or "Subheading" (case-insensitive).  Only the first such
    /// paragraph per style run is included.
    pub fn headings(&self) -> &[String] {
        &self.headings
    }

    /// All unique text fragments from the document body, in document order.
    ///
    /// Decoded from TSWP type-2001 field 3. Paragraphs are separated by `\n`;
    /// TSWP block boundaries (non-whitespace control bytes such as `\x04`) and
    /// object-replacement characters (U+FFFC) are stripped. Fragments are
    /// deduplicated.
    pub fn text_fragments(&self) -> &[String] {
        &self.text_fragments
    }

    /// Alt-text strings for all images in the document, in archive order.
    ///
    /// Decoded from type-3005 field `1.8`.
    pub fn media_descriptions(&self) -> &[String] {
        &self.media_descriptions
    }

    /// Sets the template name for encoding.
    pub fn set_template_name(&mut self, name: Option<String>) {
        self.template_name = name;
    }

    /// Replaces the text fragments for encoding.
    pub fn set_text_fragments(&mut self, fragments: Vec<String>) {
        self.text_fragments = fragments;
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

/// Builds a map from paragraph style object ID → human-readable style name.
///
/// Decodes the type-401 `DocumentStylesheet` from `Index/DocumentStylesheet.iwa`.
/// Each field-2 entry in the stylesheet maps a style key (e.g.
/// `"text-0-paragraphstyle-Title"`) to an object ID.  This function filters
/// for keys that contain `PARA_STYLE_SEP` and returns a map from object ID to
/// the name extracted after that separator (e.g. `"Title"`).
fn decode_paragraph_style_map(package: &Package) -> Result<HashMap<u64, String>, Error> {
    let bytes = package.entry_bytes(STYLESHEET_ENTRY)?;
    let archive = IwaArchive::decode(bytes)?;

    let stylesheet = archive
        .objects()
        .into_iter()
        .find(|o| o.message_type == Some(STYLESHEET_TYPE));

    let mut map = HashMap::new();

    let Some(stylesheet) = stylesheet else { return Ok(map); };
    let Ok(msg) = ProtoMessage::decode(&stylesheet.payload) else { return Ok(map); };

    for entry in msg.fields_by_number(2) {
        let Some(inner_bytes) = entry.value.as_bytes() else { continue; };
        let Ok(inner) = ProtoMessage::decode(inner_bytes) else { continue; };

        let key = inner
            .field(1)
            .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
            .and_then(|b| String::from_utf8(b).ok());
        let id = inner
            .field(2)
            .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
            .and_then(|b| ProtoMessage::decode(&b).ok())
            .and_then(|m| m.field(1).and_then(|f| f.value.as_varint()));

        if let (Some(key), Some(id)) = (key, id)
            && let Some(name) = key.split(PARA_STYLE_SEP).nth(1)
        {
            map.insert(id, name.to_owned());
        }
    }

    Ok(map)
}

/// Decodes paragraph style runs from a type-2001 object payload.
///
/// Returns a sorted list of `(byte_offset, style_name)` pairs.  `byte_offset`
/// is the offset in field 3's raw bytes where the style begins; `style_name`
/// is looked up from `style_map` and defaults to empty string when not found.
///
/// Field path for each run:
///   field 5 (bytes) → field 1 (repeated, bytes) → field 1 (uint64 offset)
///                                              → field 2 (bytes) → field 1 (uint64 style\_id)
fn decode_style_runs(payload: &[u8], style_map: &HashMap<u64, String>) -> Vec<(usize, String)> {
    let Ok(msg) = ProtoMessage::decode(payload) else { return Vec::new(); };

    let mut runs: Vec<(usize, String)> = msg
        .fields_by_number(5)
        .filter_map(|f5| {
            let bytes = f5.value.as_bytes()?;
            ProtoMessage::decode(bytes).ok()
        })
        .flat_map(|field5| {
            field5
                .fields_by_number(1)
                .filter_map(|entry| {
                    let run_bytes = entry.value.as_bytes()?;
                    let run = ProtoMessage::decode(run_bytes).ok()?;
                    let offset = usize::try_from(run.field(1)?.value.as_varint()?).ok()?;
                    let style_id = run
                        .field(2)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                        .and_then(|b| ProtoMessage::decode(&b).ok())
                        .and_then(|m| m.field(1)?.value.as_varint());
                    let name = style_id
                        .and_then(|id| style_map.get(&id))
                        .cloned()
                        .unwrap_or_default();
                    Some((offset, name))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    runs.sort_by_key(|(off, _)| *off);
    runs
}

/// Returns the paragraph style name that covers the given byte offset.
///
/// Finds the last run whose offset is ≤ `byte_offset` and returns its name.
fn style_at(runs: &[(usize, String)], byte_offset: usize) -> &str {
    runs.iter()
        .rev()
        .find(|(off, _)| *off <= byte_offset)
        .map_or("", |(_, name)| name.as_str())
}

/// Returns `true` when a paragraph style name indicates a heading.
fn is_heading_style(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("heading") || lower.contains("chapter") || lower.contains("subheading")
}

/// Returns `true` when a paragraph style name indicates the document title.
fn is_title_style(name: &str) -> bool {
    name.eq_ignore_ascii_case("title")
}

/// Decodes and classifies all TSWP text in a `Document.iwa` archive.
///
/// Returns `(title, headings, text_fragments)`.  The title is the first
/// non-empty paragraph with a "Title" paragraph style; headings are paragraphs
/// with "Heading", "Chapter", or "Subheading" styles; remaining non-empty
/// paragraphs become `text_fragments`.
fn decode_classified_text(
    archive: &IwaArchive,
    style_map: &HashMap<u64, String>,
) -> (Option<String>, Vec<String>, Vec<String>) {
    let mut title: Option<String> = None;
    let mut headings: Vec<String> = Vec::new();
    let mut fragments: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for obj in archive.objects().iter().filter(|o| o.message_type == Some(TSWP_TEXT_TYPE)) {
        let Some(raw) = ProtoMessage::decode(&obj.payload)
            .ok()
            .and_then(|msg| msg.field(3).and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec)))
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            continue;
        };

        let runs = decode_style_runs(&obj.payload, style_map);

        // Walk the raw text, tracking byte offset, splitting on \n and control chars.
        let mut para_start: usize = 0;
        let mut para_buf = String::new();

        let flush = |para_buf: &mut String,
                     para_start: usize,
                     runs: &[(usize, String)],
                     title: &mut Option<String>,
                     headings: &mut Vec<String>,
                     fragments: &mut Vec<String>,
                     seen: &mut std::collections::BTreeSet<String>| {
            let text = para_buf.trim().to_owned();
            para_buf.clear();
            if text.is_empty() || text.contains('\u{FFFC}') {
                return;
            }
            if seen.contains(&text) {
                return;
            }
            seen.insert(text.clone());
            let style = style_at(runs, para_start);
            if is_title_style(style) && title.is_none() {
                *title = Some(text);
            } else if is_heading_style(style) {
                headings.push(text);
            } else {
                fragments.push(text);
            }
        };

        let raw_bytes = raw.as_bytes();
        let mut i = 0;
        for ch in raw.chars() {
            let is_sep = ch != '\n' && ch.is_control();
            let is_newline = ch == '\n';
            if is_sep || is_newline {
                flush(
                    &mut para_buf,
                    para_start,
                    &runs,
                    &mut title,
                    &mut headings,
                    &mut fragments,
                    &mut seen,
                );
                i += ch.len_utf8();
                para_start = i;
            } else {
                para_buf.push(ch);
                i += ch.len_utf8();
            }
        }
        // flush last paragraph
        if !para_buf.is_empty() {
            let byte_len = raw_bytes.len();
            flush(
                &mut para_buf,
                para_start,
                &runs,
                &mut title,
                &mut headings,
                &mut fragments,
                &mut seen,
            );
            let _ = byte_len;
        }
    }

    (title, headings, fragments)
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
