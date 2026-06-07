use std::collections::BTreeSet;

use crate::iwa::IwaArchive;
use crate::protobuf::{ProtoMessage, read_varint};

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
        let records = extract_style_records(archive);
        let attribute_hints = extract_attribute_hints(archive);
        let strings = archive.ascii_strings(8);

        let mut identifiers = BTreeSet::new();
        let mut font_names = BTreeSet::new();
        let mut style_names = BTreeSet::new();

        for record in &records {
            if looks_like_identifier(&record.name) {
                identifiers.insert(record.name.clone());
            }
            if looks_like_style_name(&record.name) {
                style_names.insert(record.name.clone());
            }
        }

        for string in strings {
            let trimmed = string
                .trim_matches(|ch: char| matches!(ch, '"' | '$' | ':' | ',' | ';'))
                .to_owned();
            if trimmed.len() < 2 {
                continue;
            }

            if looks_like_identifier(&trimmed) {
                identifiers.insert(trimmed.clone());
            }

            if looks_like_font_name(&trimmed) {
                if let Some(suffix) = trimmed.rsplit('-').next() {
                    let suffix = suffix.trim_matches(|ch: char| !ch.is_ascii_alphabetic());
                    if looks_like_style_name(suffix) {
                        style_names.insert(suffix.to_owned());
                    }
                }
                font_names.insert(trimmed.clone());
            }

            if looks_like_style_name(&trimmed) {
                style_names.insert(trimmed);
            }
        }

        Self {
            referenced_object_ids,
            identifiers: identifiers.into_iter().collect(),
            font_names: font_names.into_iter().collect(),
            style_names: style_names.into_iter().collect(),
            records,
            attribute_hints,
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

fn extract_style_records(archive: &IwaArchive) -> Vec<StyleRecord> {
    let mut records = BTreeSet::new();
    let body = archive.body();
    let mut cursor = archive.leading_object_references_len();

    while cursor < body.len() {
        let Ok(tag) = read_varint(body, &mut cursor) else {
            break;
        };
        let wire_type = tag & 0x07;
        if wire_type != 2 {
            if skip_wire_value(body, &mut cursor, wire_type).is_err() {
                break;
            }
            continue;
        }

        let Ok(len_varint) = read_varint(body, &mut cursor) else {
            break;
        };
        let Ok(len) = usize::try_from(len_varint) else {
            break;
        };
        let Some(chunk) = body.get(cursor..cursor + len) else {
            break;
        };
        cursor += len;

        let Some(message) = ProtoMessage::decode(chunk).ok() else {
            continue;
        };
        let Some(name) = decode_message_name(&message) else {
            continue;
        };
        let Some(object_id) = decode_message_object_id(&message) else {
            continue;
        };

        records.insert(StyleRecord {
            attributes: infer_style_attributes(
                &name,
                enrich_style_attributes(body, object_id, decode_style_attributes(&message)),
            ),
            name,
            object_id: Some(object_id),
        });
    }

    records.into_iter().collect()
}

fn extract_attribute_hints(archive: &IwaArchive) -> Vec<StyleAttributes> {
    let mut hints = BTreeSet::new();
    let body = archive.body();

    for start in 0..body.len() {
        if body[start] != 0x5a {
            continue;
        }

        let mut cursor = start + 1;
        let Ok(len_varint) = read_varint(body, &mut cursor) else {
            continue;
        };
        let Ok(len) = usize::try_from(len_varint) else {
            continue;
        };
        let Some(chunk) = body.get(cursor..cursor + len) else {
            continue;
        };
        let Some(message) = ProtoMessage::decode(chunk).ok() else {
            continue;
        };
        let attributes = decode_payload_attributes(&message);
        if !attributes.is_empty() {
            hints.insert(attributes);
        }
    }

    hints.into_iter().collect()
}

fn skip_wire_value(bytes: &[u8], cursor: &mut usize, wire_type: u64) -> Result<(), ()> {
    match wire_type {
        0 => {
            read_varint(bytes, cursor).map_err(|_| ())?;
        }
        1 => {
            *cursor = cursor.checked_add(8).ok_or(())?;
            if *cursor > bytes.len() {
                return Err(());
            }
        }
        2 => {
            let len =
                usize::try_from(read_varint(bytes, cursor).map_err(|_| ())?).map_err(|_| ())?;
            *cursor = cursor.checked_add(len).ok_or(())?;
            if *cursor > bytes.len() {
                return Err(());
            }
        }
        5 => {
            *cursor = cursor.checked_add(4).ok_or(())?;
            if *cursor > bytes.len() {
                return Err(());
            }
        }
        _ => return Err(()),
    }

    Ok(())
}

fn decode_message_name(message: &ProtoMessage) -> Option<String> {
    let name = message
        .field(1)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| std::str::from_utf8(bytes).ok())?
        .trim_matches(|ch: char| matches!(ch, '"' | '$' | ':' | ',' | ';' | '*'))
        .to_owned();
    if name.len() < 3 || !name.is_ascii() {
        return None;
    }
    Some(name)
}

fn decode_message_object_id(message: &ProtoMessage) -> Option<u64> {
    message
        .field(2)
        .and_then(|field| field.value.as_bytes())
        .and_then(decode_object_reference_bytes)
        .or_else(|| {
            message
                .field(5)
                .and_then(|field| field.value.as_bytes())
                .and_then(decode_object_reference_bytes)
        })
}

fn decode_object_reference_bytes(bytes: &[u8]) -> Option<u64> {
    let message = ProtoMessage::decode(bytes).ok()?;
    match message.field(1) {
        Some(field) => field.value.as_varint().or_else(|| {
            field
                .value
                .as_bytes()
                .and_then(decode_nested_object_reference)
        }),
        None => None,
    }
}

fn decode_nested_object_reference(bytes: &[u8]) -> Option<u64> {
    let message = ProtoMessage::decode(bytes).ok()?;
    message.field(1).and_then(|field| field.value.as_varint())
}

fn enrich_style_attributes(
    body: &[u8],
    object_id: u64,
    mut attributes: StyleAttributes,
) -> StyleAttributes {
    if !attributes.is_empty() {
        return attributes;
    }

    let needle = {
        let mut bytes = vec![0x08];
        let mut value = object_id;
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                bytes.push(byte);
                break;
            }
            bytes.push(byte | 0x80);
        }
        bytes
    };

    let mut search_start = 0;
    while let Some(found) = body[search_start..]
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
    {
        let offset = search_start + found;
        let window_end = offset.saturating_add(1_000).min(body.len());
        let window = &body[offset..window_end];

        for local in 0..window.len() {
            if window[local] != 0x5a {
                continue;
            }
            let mut cursor = local + 1;
            let Ok(len_varint) = read_varint(window, &mut cursor) else {
                continue;
            };
            let Ok(len) = usize::try_from(len_varint) else {
                continue;
            };
            let Some(chunk) = window.get(cursor..cursor + len) else {
                continue;
            };
            let Some(message) = ProtoMessage::decode(chunk).ok() else {
                continue;
            };
            let candidate = decode_payload_attributes(&message);
            if !candidate.is_empty() {
                attributes = candidate;
                return attributes;
            }
        }

        search_start = offset + 1;
    }

    attributes
}

fn decode_style_attributes(message: &ProtoMessage) -> StyleAttributes {
    let payload = message
        .field(11)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| ProtoMessage::decode(bytes).ok());

    payload
        .as_ref()
        .map_or_else(StyleAttributes::default, decode_payload_attributes)
}

fn decode_payload_attributes(payload: &ProtoMessage) -> StyleAttributes {
    let font_name = payload
        .field(5)
        .and_then(|field| field.value.as_bytes())
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(ToOwned::to_owned);
    let font_size = payload.field(3).and_then(|field| match field.value {
        crate::ProtoValue::Fixed32(bits) => Some(OrderedF32(f32::from_bits(bits))),
        _ => None,
    });

    StyleAttributes {
        font_name,
        font_size,
        bold: None,
        italic: None,
        underline: None,
        strikethrough: None,
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
