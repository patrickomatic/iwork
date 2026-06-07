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
}

impl StylesheetCatalog {
    pub fn from_archive(archive: &IwaArchive) -> Self {
        let referenced_object_ids = archive.leading_object_references();
        let records = extract_style_records(archive);
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StyleRecord {
    pub name: String,
    pub object_id: Option<u64>,
}

fn extract_style_records(archive: &IwaArchive) -> Vec<StyleRecord> {
    let mut records = BTreeSet::new();
    let body = archive.body();

    for start in 0..body.len() {
        let Some(record) = decode_style_record_at(&body[start..]) else {
            continue;
        };
        records.insert(record);
    }

    records.into_iter().collect()
}

fn decode_style_record_at(bytes: &[u8]) -> Option<StyleRecord> {
    let mut cursor = 0;
    let tag = read_varint(bytes, &mut cursor).ok()?;
    if tag != 10 {
        return None;
    }

    let name_len = usize::try_from(read_varint(bytes, &mut cursor).ok()?).ok()?;
    let name_end = cursor.checked_add(name_len)?;
    let name = std::str::from_utf8(bytes.get(cursor..name_end)?)
        .ok()?
        .trim_matches(|ch: char| matches!(ch, '"' | '$' | ':' | ',' | ';' | '*'))
        .to_owned();
    if name.len() < 3 || !name.is_ascii() {
        return None;
    }
    cursor = name_end;

    let mut object_id = None;
    while cursor < bytes.len() {
        let field_tag = read_varint(bytes, &mut cursor).ok()?;
        let field_number = field_tag >> 3;
        let wire_type = field_tag & 0x07;
        match (field_number, wire_type) {
            (2 | 5, 2) => {
                let len = usize::try_from(read_varint(bytes, &mut cursor).ok()?).ok()?;
                let end = cursor.checked_add(len)?;
                let value = bytes.get(cursor..end)?;
                object_id = decode_object_reference_bytes(value);
                if object_id.is_some() {
                    break;
                }
                cursor = end;
            }
            (_, 0) => {
                read_varint(bytes, &mut cursor).ok()?;
            }
            (_, 1) => {
                cursor = cursor.checked_add(8)?;
            }
            (_, 2) => {
                let len = usize::try_from(read_varint(bytes, &mut cursor).ok()?).ok()?;
                cursor = cursor.checked_add(len)?;
            }
            (_, 5) => {
                cursor = cursor.checked_add(4)?;
            }
            _ => return None,
        }
    }

    if object_id.is_none() {
        return None;
    }

    Some(StyleRecord { name, object_id })
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
