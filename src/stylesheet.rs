use std::collections::{BTreeSet, HashMap};

use crate::iwa::{IwaArchive, IwaObject};
use crate::protobuf::{ProtoMessage, ProtoValue};

/// Message type of the `DocumentStylesheet` root object.
const TYPE_DOCUMENT_STYLESHEET: u64 = 401;

/// Field on `DocumentStylesheet` that holds the name → object-id registry.
/// Each occurrence is `{field 1: identifier_string, field 2: {field 1: object_id}}`.
const DS_FIELD_STYLE_ENTRIES: u32 = 2;
const STYLE_ENTRY_NAME: u32 = 1;
const STYLE_ENTRY_REF: u32 = 2;
const REF_FIELD_ID: u32 = 1;

/// Field on a style object (type 2022 / 2025) that holds text attributes.
const STYLE_FIELD_TEXT_ATTRS: u32 = 11;
/// Within the text-attributes sub-message:
const TEXT_ATTR_BOLD: u32 = 1;
const TEXT_ATTR_ITALIC: u32 = 2;
const TEXT_ATTR_FONT_SIZE: u32 = 3;
const TEXT_ATTR_FONT_NAME: u32 = 5;
/// Field 13 is observed as 1 on styles that correspond to hyperlinks across Pages
/// documents; this is our best structural candidate for underline. Not cross-validated
/// against a fixture with explicit underline=on / underline=off contrast.
const TEXT_ATTR_UNDERLINE: u32 = 13;
/// Sub-message at field 7 carries the text foreground color as sRGB components.
const TEXT_ATTR_COLOR: u32 = 7;
const COLOR_R: u32 = 3;
const COLOR_G: u32 = 4;
const COLOR_B: u32 = 5;
const COLOR_A: u32 = 6;

/// Top-level field on a style object (type 2022) that holds paragraph/cell attributes.
const STYLE_FIELD_CELL_ATTRS: u32 = 12;
/// `CellStyle.field 1` — text alignment enum:
/// 0 = auto, 1 = left, 2 = center, 4 = right.
/// Cross-validated by corpus statistics; left/center/right are the three non-trivial values.
const CELL_ATTR_ALIGNMENT: u32 = 1;

/// Field on a style object that holds the display name (type 2022 field 1.1).
const STYLE_FIELD_BASE: u32 = 1;
const BASE_FIELD_NAME: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StylesheetCatalog {
    pub referenced_object_ids: Vec<u64>,
    pub identifiers: Vec<String>,
    pub font_names: Vec<String>,
    pub style_names: Vec<String>,
    pub records: Vec<StyleRecord>,
    pub attribute_hints: Vec<StyleAttributes>,
}

impl StylesheetCatalog {
    pub fn from_archive(archive: &IwaArchive) -> Self {
        let referenced_object_ids = archive.leading_object_references();

        // Build a map from object id → object for fast lookup.
        let objects = archive.objects();
        let by_id: HashMap<u64, &IwaObject> = objects
            .iter()
            .filter_map(|obj| Some((obj.identifier?, obj)))
            .collect();

        // Decode the style name registry from the DocumentStylesheet root.
        let registry = decode_style_registry(archive, &by_id);

        // Build StyleRecords from the registry.
        let records: BTreeSet<StyleRecord> = registry
            .iter()
            .map(|(name, obj)| {
                let text_attrs = decode_text_attributes(obj);
                let inferred = infer_style_attributes(name, text_attrs);
                StyleRecord {
                    name: name.clone(),
                    object_id: obj.identifier,
                    attributes: inferred,
                }
            })
            .collect();

        // Derive aggregate lists.
        let mut identifiers = BTreeSet::new();
        let mut font_names_set = BTreeSet::new();
        let mut style_names_set = BTreeSet::new();
        for rec in &records {
            if looks_like_identifier(&rec.name) {
                identifiers.insert(rec.name.clone());
            }
            if looks_like_style_name(&rec.name) {
                style_names_set.insert(rec.name.clone());
            }
            if let Some(ref f) = rec.attributes.font_name
                && looks_like_font_name(f)
            {
                font_names_set.insert(f.clone());
            }
        }

        // Keep any display names that look like style names.
        for obj in by_id.values() {
            if let Some(display) = decode_display_name(obj)
                && looks_like_style_name(&display)
            {
                style_names_set.insert(display);
            }
        }

        // Attribute hints: unique attribute sets seen across all records.
        let hints: BTreeSet<StyleAttributes> = records
            .iter()
            .map(|r| r.attributes.clone())
            .filter(|a| !a.is_empty())
            .collect();

        Self {
            referenced_object_ids,
            identifiers: identifiers.into_iter().collect(),
            font_names: font_names_set.into_iter().collect(),
            style_names: style_names_set.into_iter().collect(),
            records: records.into_iter().collect(),
            attribute_hints: hints.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StyleRecord {
    pub name: String,
    pub object_id: Option<u64>,
    pub attributes: StyleAttributes,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct StyleAttributes {
    pub font_name: Option<String>,
    pub font_size: Option<OrderedF32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
    pub color: Option<Color>,
    pub alignment: Option<TextAlignment>,
}

/// sRGB text foreground color with alpha, all channels in \[0, 1\].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Color {
    pub r: OrderedF32,
    pub g: OrderedF32,
    pub b: OrderedF32,
    pub a: OrderedF32,
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextAlignment {
    Auto,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OrderedF32(pub f32);

impl Eq for OrderedF32 {}

impl PartialOrd for OrderedF32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedF32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// Parses the `DocumentStylesheet` root to build an identifier → object map.
fn decode_style_registry<'a>(
    archive: &'a IwaArchive,
    by_id: &HashMap<u64, &'a IwaObject>,
) -> Vec<(String, &'a IwaObject)> {
    let objects = archive.objects();
    let root = objects
        .iter()
        .find(|obj| obj.message_type == Some(TYPE_DOCUMENT_STYLESHEET));

    let Some(root) = root else {
        return Vec::new();
    };

    let Ok(message) = ProtoMessage::decode(&root.payload) else {
        return Vec::new();
    };

    message
        .fields_by_number(DS_FIELD_STYLE_ENTRIES)
        .filter_map(|field| {
            let entry = field
                .value
                .as_bytes()
                .and_then(|b| ProtoMessage::decode(b).ok())?;
            let name = entry
                .field(STYLE_ENTRY_NAME)
                .and_then(|f| f.value.as_bytes())
                .and_then(|b| std::str::from_utf8(b).ok())
                .map(ToOwned::to_owned)?;
            let object_id = entry
                .field(STYLE_ENTRY_REF)
                .and_then(|f| f.value.as_bytes())
                .and_then(|b| ProtoMessage::decode(b).ok())
                .and_then(|r| r.field(REF_FIELD_ID).and_then(|f| f.value.as_varint()))?;
            let obj = *by_id.get(&object_id)?;
            Some((name, obj))
        })
        .collect()
}

/// Decodes text attributes (bold, italic, font, size, underline) from a style object.
///
/// Type 2022 and 2025 both carry a text-attribute sub-message at field 11.
fn decode_text_attributes(obj: &IwaObject) -> StyleAttributes {
    let Ok(message) = ProtoMessage::decode(&obj.payload) else {
        return StyleAttributes::default();
    };

    let Some(text_attrs) = message
        .field(STYLE_FIELD_TEXT_ATTRS)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| ProtoMessage::decode(b).ok())
    else {
        return StyleAttributes::default();
    };

    let bold = text_attrs
        .field(TEXT_ATTR_BOLD)
        .and_then(|f| f.value.as_varint())
        .map(|v| v != 0);
    let italic = text_attrs
        .field(TEXT_ATTR_ITALIC)
        .and_then(|f| f.value.as_varint())
        .map(|v| v != 0);
    let font_size = text_attrs.field(TEXT_ATTR_FONT_SIZE).and_then(|f| {
        if let crate::protobuf::ProtoValue::Fixed32(bits) = f.value {
            let size = f32::from_bits(bits);
            if size > 0.0 && size.is_finite() {
                return Some(OrderedF32(size));
            }
        }
        None
    });
    let font_name = text_attrs
        .field(TEXT_ATTR_FONT_NAME)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(ToOwned::to_owned);
    let underline = text_attrs
        .field(TEXT_ATTR_UNDERLINE)
        .and_then(|f| f.value.as_varint())
        .map(|v| v != 0);

    let color = text_attrs
        .field(TEXT_ATTR_COLOR)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| ProtoMessage::decode(b).ok())
        .and_then(|msg| decode_color_message(&msg));

    let alignment = message
        .field(STYLE_FIELD_CELL_ATTRS)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| ProtoMessage::decode(b).ok())
        .and_then(|cell| cell.field(CELL_ATTR_ALIGNMENT).and_then(|f| f.value.as_varint()))
        .and_then(|v| match v {
            0 => Some(TextAlignment::Auto),
            1 => Some(TextAlignment::Left),
            2 => Some(TextAlignment::Center),
            4 => Some(TextAlignment::Right),
            _ => None,
        });

    StyleAttributes {
        bold,
        italic,
        font_size,
        font_name,
        underline,
        strikethrough: None,
        color,
        alignment,
    }
}

fn decode_color_message(msg: &ProtoMessage) -> Option<Color> {
    let read_f32 = |n: u32| -> Option<OrderedF32> {
        msg.field(n).and_then(|f| {
            if let ProtoValue::Fixed32(bits) = f.value {
                Some(OrderedF32(f32::from_bits(bits)))
            } else {
                None
            }
        })
    };
    let r = read_f32(COLOR_R)?;
    let g = read_f32(COLOR_G)?;
    let b = read_f32(COLOR_B)?;
    let a = read_f32(COLOR_A).unwrap_or(OrderedF32(1.0));
    Some(Color { r, g, b, a })
}

/// Extracts the display name from a style object's `field 1.1`, if present.
fn decode_display_name(obj: &IwaObject) -> Option<String> {
    let message = ProtoMessage::decode(&obj.payload).ok()?;
    let base = message
        .field(STYLE_FIELD_BASE)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| ProtoMessage::decode(b).ok())?;
    let name = base
        .field(BASE_FIELD_NAME)
        .and_then(|f| f.value.as_bytes())
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(ToOwned::to_owned)?;
    if name.len() >= 3 && name.is_ascii() {
        Some(name)
    } else {
        None
    }
}

fn infer_style_attributes(name: &str, mut attributes: StyleAttributes) -> StyleAttributes {
    let lowercase_name = name.to_ascii_lowercase();
    let lowercase_font = attributes
        .font_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if attributes.bold.is_none()
        && (lowercase_name.contains("bold") || lowercase_font.contains("bold"))
    {
        attributes.bold = Some(true);
    }
    if attributes.italic.is_none() && lowercase_name.contains("italic") {
        attributes.italic = Some(true);
    }
    if attributes.underline.is_none() && lowercase_name.contains("underline") {
        attributes.underline = Some(true);
    }
    if attributes.strikethrough.is_none() && lowercase_name.contains("strikethrough") {
        attributes.strikethrough = Some(true);
    }

    attributes
}

impl StyleAttributes {
    fn is_empty(&self) -> bool {
        self.font_name.is_none()
            && self.font_size.is_none()
            && self.bold.is_none()
            && self.italic.is_none()
            && self.underline.is_none()
            && self.strikethrough.is_none()
            && self.color.is_none()
            && self.alignment.is_none()
    }
}

fn looks_like_identifier(value: &str) -> bool {
    value.contains('-') || value.contains('_')
}

fn looks_like_font_name(value: &str) -> bool {
    value.chars().any(char::is_lowercase)
        && value.chars().any(char::is_uppercase)
        && (value.contains("Neue")
            || value.contains("Helvetica")
            || value.contains("Times")
            || value.contains("Arial")
            || value.contains("Avenir")
            || value.contains("Courier"))
}

fn looks_like_style_name(value: &str) -> bool {
    value.split_whitespace().count() <= 4
        && value
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_uppercase())
        && !looks_like_font_name(value)
        && !looks_like_identifier(value)
}
