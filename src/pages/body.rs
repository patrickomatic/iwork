use crate::Error;
use crate::iwa::IwaArchive;
use crate::package::Package;
use crate::protobuf::{ProtoMessage, ProtoValue};
use std::collections::{BTreeSet, HashMap};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";
const STYLESHEET_ENTRY: &str = "Index/DocumentStylesheet.iwa";

const WP_BODY_TYPE: u64 = 10001;
const STYLESHEET_TYPE: u64 = 401;
const TSWP_TEXT_TYPE: u64 = 2001;
const NAMED_CHAR_STYLE_TYPE: u64 = 2021;
const CHAR_STYLE_TYPE: u64 = 2022;
const LIST_STYLE_TYPE: u64 = 2023;
const MEDIA_TYPE: u64 = 3005;

const PARA_STYLE_SEP: &str = "-paragraphstyle-";

/// Character-level text formatting for a [`TextRun`].
///
/// All fields are optional; `None` means the attribute is inherited from the
/// paragraph's default style rather than explicitly set on this run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextFormatting {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// Whether the run is struck through.
    ///
    /// Decoded from type-2021 named char style "Strikethrough" via type-2001
    /// field 8. The protobuf field number for encoding is not yet grounded, so
    /// this attribute is read-only in generated packages.
    pub strikethrough: Option<bool>,
    pub font_name: Option<String>,
    /// Font size in typographic points.
    pub font_size_pt: Option<f32>,
}

impl TextFormatting {
    pub(crate) fn is_default(&self) -> bool {
        self.bold.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
            && self.strikethrough.is_none()
            && self.font_name.is_none()
            && self.font_size_pt.is_none()
    }
}

/// A contiguous run of text sharing the same character-level formatting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub formatting: TextFormatting,
}

/// A paragraph from the Pages document body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paragraph {
    /// The paragraph style name (e.g. `"Title"`, `"Heading"`, `"Body"`).
    ///
    /// Sourced from the paragraph style referenced in type-2001 field 5, looked
    /// up in the `DocumentStylesheet.iwa` type-401 registry.
    pub style_name: String,
    /// The list style name for this paragraph, if it belongs to a list.
    ///
    /// `None` for non-list paragraphs (including those with list style "None").
    /// Typical values: `"Bullet"`, `"Bullet 2"`, `"Numbered"`, `"Lettered"`.
    /// Sourced from type-2001 field 7 → type-2023 list style objects.
    pub list_style: Option<String>,
    /// The text runs that make up this paragraph's content.
    pub runs: Vec<TextRun>,
}

impl Paragraph {
    /// Returns the paragraph's full text by concatenating all runs.
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    pub(crate) template_name: Option<String>,
    /// Structured paragraph list — the primary semantic model.
    pub(crate) paragraphs: Vec<Paragraph>,
    // Derived caches kept for backward-compatible accessors:
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

        let para_style_map = decode_paragraph_style_map(package)?;
        let char_style_map = decode_char_style_map(package);
        let list_style_map = decode_list_style_map(package);
        let named_char_style_map = decode_named_char_style_map(package);
        let paragraphs = decode_paragraphs(
            &archive,
            &para_style_map,
            &char_style_map,
            &list_style_map,
            &named_char_style_map,
        );
        let media_descriptions = decode_media_descriptions(&archive);

        let (title, headings, text_fragments) = derive_classified_text(&paragraphs);

        Ok(Self {
            template_name,
            paragraphs,
            title,
            headings,
            text_fragments,
            media_descriptions,
        })
    }

    /// The iWork template identifier used to create this document, if present.
    ///
    /// Sourced from type-10001 field `1.3`.
    pub fn template_name(&self) -> Option<&str> {
        self.template_name.as_deref()
    }

    /// The document title, sourced from the first paragraph whose style name is
    /// `"Title"` (case-insensitive). Returns `None` when absent.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Heading paragraphs in document order.
    ///
    /// Includes paragraphs whose style name contains "Heading", "Chapter", or
    /// "Subheading". Deduplicated.
    pub fn headings(&self) -> &[String] {
        &self.headings
    }

    /// All non-title, non-heading paragraphs in document order.
    ///
    /// Deduplicated; each entry is the trimmed concatenation of the paragraph's
    /// text runs.
    pub fn text_fragments(&self) -> &[String] {
        &self.text_fragments
    }

    /// Alt-text strings for all images in the document, in archive order.
    pub fn media_descriptions(&self) -> &[String] {
        &self.media_descriptions
    }

    /// All paragraphs in document order, with style names and character runs.
    ///
    /// This is the richest view of the document — use it when you need
    /// paragraph-level style names or character-level formatting.
    pub fn paragraphs(&self) -> &[Paragraph] {
        &self.paragraphs
    }

    /// Sets the template name for encoding.
    pub fn set_template_name(&mut self, name: Option<String>) {
        self.template_name = name;
    }

    /// Replaces the document content with the given paragraphs.
    ///
    /// Recomputes `title()`, `headings()`, and `text_fragments()` from the new
    /// paragraph list.
    pub fn set_paragraphs(&mut self, paragraphs: Vec<Paragraph>) {
        let (title, headings, text_fragments) = derive_classified_text(&paragraphs);
        self.title = title;
        self.headings = headings;
        self.text_fragments = text_fragments;
        self.paragraphs = paragraphs;
    }

    /// Convenience setter: replaces the document with plain body paragraphs.
    ///
    /// Each fragment becomes a `Paragraph` with `style_name = "Body"` and a
    /// single unstyled run. Clears title and headings.
    pub fn set_text_fragments(&mut self, fragments: Vec<String>) {
        self.title = None;
        self.headings.clear();
        self.text_fragments.clone_from(&fragments);
        self.paragraphs = fragments
            .into_iter()
            .map(|text| Paragraph {
                style_name: "Body".to_owned(),
                list_style: None,
                runs: vec![TextRun {
                    text,
                    formatting: TextFormatting::default(),
                }],
            })
            .collect();
    }
}

/// Computes `title`, `headings`, and `text_fragments` from a paragraph list.
///
/// Preserves the deduplication behaviour of the previous implementation: the
/// same text string is not emitted twice across any of the three buckets.
pub(crate) fn derive_classified_text(
    paragraphs: &[Paragraph],
) -> (Option<String>, Vec<String>, Vec<String>) {
    let mut title: Option<String> = None;
    let mut headings: Vec<String> = Vec::new();
    let mut text_fragments: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for para in paragraphs {
        let trimmed = para.text();
        let trimmed = trimmed.trim().to_owned();
        if trimmed.is_empty() {
            continue;
        }
        if seen.contains(&trimmed) {
            continue;
        }
        seen.insert(trimmed.clone());

        if is_title_style(&para.style_name) && title.is_none() {
            title = Some(trimmed);
        } else if is_heading_style(&para.style_name) {
            headings.push(trimmed);
        } else {
            text_fragments.push(trimmed);
        }
    }

    (title, headings, text_fragments)
}

fn is_title_style(name: &str) -> bool {
    name.eq_ignore_ascii_case("title")
}

fn is_heading_style(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("heading") || lower.contains("chapter") || lower.contains("subheading")
}

// ─── Decoder ─────────────────────────────────────────────────────────────────

fn decode_template_name(archive: &IwaArchive) -> Option<String> {
    archive
        .objects()
        .into_iter()
        .find(|obj| obj.message_type == Some(WP_BODY_TYPE))
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
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

/// Builds a map from paragraph style object ID → human-readable style name.
///
/// Decodes the type-401 `DocumentStylesheet` from `Index/DocumentStylesheet.iwa`.
/// Each field-2 entry maps a key like `"text-0-paragraphstyle-Title"` to an
/// object ID. Returns `id → "Title"` (the part after `"-paragraphstyle-"`).
fn decode_paragraph_style_map(package: &Package) -> Result<HashMap<u64, String>, Error> {
    let bytes = package.entry_bytes(STYLESHEET_ENTRY)?;
    let archive = IwaArchive::decode(bytes)?;

    let mut map = HashMap::new();

    let Some(stylesheet) = archive
        .objects()
        .into_iter()
        .find(|o| o.message_type == Some(STYLESHEET_TYPE))
    else {
        return Ok(map);
    };

    let Ok(msg) = ProtoMessage::decode(&stylesheet.payload) else {
        return Ok(map);
    };

    for entry in msg.fields_by_number(2) {
        let Some(inner_bytes) = entry.value.as_bytes() else {
            continue;
        };
        let Ok(inner) = ProtoMessage::decode(inner_bytes) else {
            continue;
        };

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

/// Builds a map from character style object ID → `TextFormatting`.
///
/// Scans type-2022 objects in `DocumentStylesheet.iwa` and reads the
/// text-attribute sub-message at field 11. Used for char style runs stored in
/// type-2001 field 7 by our own encoder (synthetic packages only).
fn decode_char_style_map(package: &Package) -> HashMap<u64, TextFormatting> {
    let Ok(bytes) = package.entry_bytes(STYLESHEET_ENTRY) else {
        return HashMap::new();
    };
    let Ok(archive) = IwaArchive::decode(bytes) else {
        return HashMap::new();
    };

    archive
        .objects()
        .into_iter()
        .filter(|obj| obj.message_type == Some(CHAR_STYLE_TYPE))
        .filter_map(|obj| {
            let id = obj.identifier?;
            let fmt = decode_text_formatting(&obj.payload);
            Some((id, fmt))
        })
        .collect()
}

/// Decodes `TextFormatting` from a style object payload by reading field 11.
fn decode_text_formatting(payload: &[u8]) -> TextFormatting {
    let Ok(msg) = ProtoMessage::decode(payload) else {
        return TextFormatting::default();
    };
    let Some(attrs) = msg
        .field(11)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| ProtoMessage::decode(b).ok())
    else {
        return TextFormatting::default();
    };

    let bold = attrs
        .field(1)
        .and_then(|f| f.value.as_varint())
        .map(|v| v != 0);
    let italic = attrs
        .field(2)
        .and_then(|f| f.value.as_varint())
        .map(|v| v != 0);
    let font_size_pt = attrs.field(3).and_then(|f| {
        if let ProtoValue::Fixed32(bits) = f.value {
            let size = f32::from_bits(bits);
            if size > 0.0 && size.is_finite() {
                Some(size)
            } else {
                None
            }
        } else {
            None
        }
    });
    let font_name = attrs
        .field(5)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(ToOwned::to_owned);
    let underline = attrs
        .field(13)
        .and_then(|f| f.value.as_varint())
        .map(|v| v != 0);

    TextFormatting {
        bold,
        italic,
        underline,
        strikethrough: None,
        font_name,
        font_size_pt,
    }
}

/// Builds a map from list style object ID → list style name.
///
/// Scans `DocumentStylesheet.iwa` for type-2023 list style objects and reads
/// the name string at `field 1.1`. Paragraphs whose field-7 run ID is in this
/// map are list items; the name "None" means no active list.
fn decode_list_style_map(package: &Package) -> HashMap<u64, String> {
    let Ok(bytes) = package.entry_bytes(STYLESHEET_ENTRY) else {
        return HashMap::new();
    };
    let Ok(archive) = IwaArchive::decode(bytes) else {
        return HashMap::new();
    };

    archive
        .objects()
        .into_iter()
        .filter(|obj| obj.message_type == Some(LIST_STYLE_TYPE))
        .filter_map(|obj| {
            let id = obj.identifier?;
            let msg = ProtoMessage::decode(&obj.payload).ok()?;
            let name = msg
                .field(1)
                .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                .and_then(|b| ProtoMessage::decode(&b).ok())
                .and_then(|inner| {
                    inner
                        .field(1)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                })
                .and_then(|b| String::from_utf8(b).ok())?;
            Some((id, name))
        })
        .collect()
}

/// Decodes paragraph style runs from a type-2001 payload (field 5).
///
/// Returns sorted `(byte_offset, style_name)` pairs. The style name is looked
/// up via `style_map`; absent IDs produce an empty name.
fn decode_style_runs(payload: &[u8], style_map: &HashMap<u64, String>) -> Vec<(usize, String)> {
    let Ok(msg) = ProtoMessage::decode(payload) else {
        return Vec::new();
    };

    let mut runs: Vec<(usize, String)> = msg
        .fields_by_number(5)
        .filter_map(|f5| {
            f5.value
                .as_bytes()
                .and_then(|b| ProtoMessage::decode(b).ok())
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

/// Decodes style id runs from a type-2001 payload at the given field number.
///
/// Returns sorted `(byte_offset, style_object_id)` pairs. Wire structure:
/// outer field is a length-delimited blob; inside, repeated field 1 entries
/// each encode `{field 1: byte_offset, field 2: {field 1: style_id}}`.
///
/// Used for field 7 (list style runs) and field 8 (named char style runs).
fn decode_id_runs_raw(payload: &[u8], field_number: u32) -> Vec<(usize, u64)> {
    let Ok(msg) = ProtoMessage::decode(payload) else {
        return Vec::new();
    };

    let mut runs: Vec<(usize, u64)> = msg
        .fields_by_number(field_number)
        .filter_map(|f| {
            f.value
                .as_bytes()
                .and_then(|b| ProtoMessage::decode(b).ok())
        })
        .flat_map(|blob| {
            blob.fields_by_number(1)
                .filter_map(|entry| {
                    let run_bytes = entry.value.as_bytes()?;
                    let run = ProtoMessage::decode(run_bytes).ok()?;
                    let offset = usize::try_from(run.field(1)?.value.as_varint()?).ok()?;
                    let style_id = run
                        .field(2)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                        .and_then(|b| ProtoMessage::decode(&b).ok())
                        .and_then(|m| m.field(1)?.value.as_varint())?;
                    Some((offset, style_id))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    runs.sort_by_key(|(off, _)| *off);
    runs
}

/// Builds a map from named char style object ID (type-2021) → `TextFormatting`.
///
/// Named char styles like "Emphasis", "Italic", "Link", and "Strikethrough" are
/// referenced by type-2001 field 8 in real Pages documents.
fn decode_named_char_style_map(package: &Package) -> HashMap<u64, TextFormatting> {
    let Ok(bytes) = package.entry_bytes(STYLESHEET_ENTRY) else {
        return HashMap::new();
    };
    let Ok(archive) = IwaArchive::decode(bytes) else {
        return HashMap::new();
    };

    archive
        .objects()
        .into_iter()
        .filter(|obj| obj.message_type == Some(NAMED_CHAR_STYLE_TYPE))
        .filter_map(|obj| {
            let id = obj.identifier?;
            let msg = ProtoMessage::decode(&obj.payload).ok()?;
            let name = msg
                .field(1)
                .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                .and_then(|b| ProtoMessage::decode(&b).ok())
                .and_then(|inner| {
                    inner
                        .field(1)
                        .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
                })
                .and_then(|b| String::from_utf8(b).ok())?;
            Some((id, named_char_style_formatting(&name)))
        })
        .collect()
}

/// Maps a type-2021 named char style name to its `TextFormatting` equivalent.
fn named_char_style_formatting(name: &str) -> TextFormatting {
    match name {
        "Emphasis" | "Italic" => TextFormatting {
            italic: Some(true),
            ..TextFormatting::default()
        },
        "Link" => TextFormatting {
            underline: Some(true),
            ..TextFormatting::default()
        },
        "Strikethrough" => TextFormatting {
            strikethrough: Some(true),
            ..TextFormatting::default()
        },
        _ => TextFormatting::default(),
    }
}

/// Merges two `TextFormatting` values: overlay fields take precedence over base.
fn merge_formatting(base: TextFormatting, overlay: TextFormatting) -> TextFormatting {
    TextFormatting {
        bold: overlay.bold.or(base.bold),
        italic: overlay.italic.or(base.italic),
        underline: overlay.underline.or(base.underline),
        strikethrough: overlay.strikethrough.or(base.strikethrough),
        font_name: overlay.font_name.or(base.font_name),
        font_size_pt: overlay.font_size_pt.or(base.font_size_pt),
    }
}

/// Returns the style name whose run covers `byte_offset` (last run ≤ offset).
fn style_at(runs: &[(usize, String)], byte_offset: usize) -> &str {
    runs.iter()
        .rev()
        .find(|(off, _)| *off <= byte_offset)
        .map_or("", |(_, name)| name.as_str())
}

/// Returns the char style object ID whose run covers `byte_offset`.
fn char_id_at(char_runs: &[(usize, u64)], byte_offset: usize) -> Option<u64> {
    char_runs
        .iter()
        .rev()
        .find(|(off, _)| *off <= byte_offset)
        .map(|(_, id)| *id)
}

/// Splits a paragraph's raw text into [`TextRun`]s.
///
/// Split points come from two sources:
/// - `char_runs` (type-2001 field 7): char style runs used by our own encoder
///   (synthetic packages). IDs resolved via `char_style_map` (type-2022 objects).
/// - `named_char_runs` (type-2001 field 8): named char style runs from real Pages
///   documents. IDs resolved via `named_char_style_map` (type-2021 objects).
///
/// Formatting at each split is the merge of both sources; the named char style
/// overlay takes precedence for any field it sets.
fn build_text_runs(
    raw_text: &str,
    para_start: usize,
    char_runs: &[(usize, u64)],
    char_style_map: &HashMap<u64, TextFormatting>,
    named_char_runs: &[(usize, u64)],
    named_char_style_map: &HashMap<u64, TextFormatting>,
) -> Vec<TextRun> {
    let para_end = para_start + raw_text.len();

    // Collect split boundaries from both style run sources.
    let mut boundaries: Vec<usize> = vec![para_start];
    for (off, _) in char_runs.iter().chain(named_char_runs.iter()) {
        if *off > para_start && *off < para_end {
            boundaries.push(*off);
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut runs = Vec::new();
    for i in 0..boundaries.len() {
        let run_start = boundaries[i];
        let run_end = boundaries.get(i + 1).copied().unwrap_or(para_end);

        let base_fmt = char_id_at(char_runs, run_start)
            .and_then(|id| char_style_map.get(&id))
            .cloned()
            .unwrap_or_default();
        let named_fmt = char_id_at(named_char_runs, run_start)
            .and_then(|id| named_char_style_map.get(&id))
            .cloned()
            .unwrap_or_default();
        let formatting = merge_formatting(base_fmt, named_fmt);

        let start_in_para = run_start - para_start;
        let end_in_para = run_end - para_start;

        if start_in_para <= raw_text.len()
            && end_in_para <= raw_text.len()
            && raw_text.is_char_boundary(start_in_para)
            && raw_text.is_char_boundary(end_in_para)
        {
            let text = raw_text[start_in_para..end_in_para].to_owned();
            if !text.is_empty() {
                runs.push(TextRun { text, formatting });
            }
        }
    }

    if runs.is_empty() {
        let text = raw_text.trim().to_owned();
        if !text.is_empty() {
            runs.push(TextRun {
                text,
                formatting: TextFormatting::default(),
            });
        }
    }

    runs
}

/// Decodes all TSWP text objects in `archive` into a [`Paragraph`] list.
/// Bundles the three style maps used during paragraph decoding.
struct StyleMaps<'a> {
    para: &'a HashMap<u64, String>,
    char_fmt: &'a HashMap<u64, TextFormatting>,
    named_char_fmt: &'a HashMap<u64, TextFormatting>,
    list: &'a HashMap<u64, String>,
}

fn decode_paragraphs(
    archive: &IwaArchive,
    para_style_map: &HashMap<u64, String>,
    char_style_map: &HashMap<u64, TextFormatting>,
    list_style_map: &HashMap<u64, String>,
    named_char_style_map: &HashMap<u64, TextFormatting>,
) -> Vec<Paragraph> {
    let maps = StyleMaps {
        para: para_style_map,
        char_fmt: char_style_map,
        named_char_fmt: named_char_style_map,
        list: list_style_map,
    };
    let mut paragraphs = Vec::new();

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

        let para_runs = decode_style_runs(&obj.payload, maps.para);
        let char_runs = decode_id_runs_raw(&obj.payload, 7);
        let named_char_runs = decode_id_runs_raw(&obj.payload, 8);

        let mut para_start = 0usize;
        let mut i = 0usize;
        let mut para_buf = String::new();

        for ch in raw.chars() {
            if ch.is_control() || ch == '\n' {
                push_paragraph(&para_buf, para_start, &para_runs, &char_runs, &named_char_runs, &maps, &mut paragraphs);
                para_buf.clear();
                i += ch.len_utf8();
                para_start = i;
            } else {
                para_buf.push(ch);
                i += ch.len_utf8();
            }
        }
        if !para_buf.is_empty() {
            push_paragraph(&para_buf, para_start, &para_runs, &char_runs, &named_char_runs, &maps, &mut paragraphs);
        }
    }

    paragraphs
}

fn push_paragraph(
    raw_text: &str,
    para_start: usize,
    para_runs: &[(usize, String)],
    char_runs: &[(usize, u64)],
    named_char_runs: &[(usize, u64)],
    maps: &StyleMaps<'_>,
    paragraphs: &mut Vec<Paragraph>,
) {
    if raw_text.trim().is_empty() || raw_text.trim().contains('\u{FFFC}') {
        return;
    }
    let style_name = style_at(para_runs, para_start).to_owned();
    let list_style = char_id_at(char_runs, para_start)
        .and_then(|id| maps.list.get(&id))
        .filter(|name| name.as_str() != "None")
        .cloned();
    let runs = build_text_runs(
        raw_text,
        para_start,
        char_runs,
        maps.char_fmt,
        named_char_runs,
        maps.named_char_fmt,
    );
    if !runs.is_empty() {
        paragraphs.push(Paragraph {
            style_name,
            list_style,
            runs,
        });
    }
}

/// Reads alt-text from all type-3005 media objects in `Document.iwa`.
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
