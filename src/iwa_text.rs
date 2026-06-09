use std::collections::BTreeSet;

use crate::iwa::IwaArchive;
use crate::protobuf::{ProtoMessage, ProtoValue, read_varint};

const MAX_RECURSION_DEPTH: usize = 16;

pub(crate) fn extract_utf8_fields(archive: &IwaArchive) -> Vec<String> {
    let mut collector = TextCollector::default();
    let body = archive.body();
    let start = archive.leading_object_references_len();
    let Some(fields) = body.get(start..) else {
        return Vec::new();
    };

    collector.collect_field_stream(fields, 0);
    collector.into_strings()
}

#[derive(Default)]
struct TextCollector {
    seen: BTreeSet<String>,
    strings: Vec<String>,
}

impl TextCollector {
    fn into_strings(self) -> Vec<String> {
        self.strings
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> bool {
        let Some(text) = normalize_utf8_field(bytes) else {
            return false;
        };
        if self.seen.insert(text.clone()) {
            self.strings.push(text);
        }
        true
    }

    fn collect_field_stream(&mut self, bytes: &[u8], depth: usize) {
        if depth >= MAX_RECURSION_DEPTH {
            return;
        }

        let mut cursor = 0;
        while cursor < bytes.len() {
            let Ok(tag) = read_varint(bytes, &mut cursor) else {
                return;
            };
            if tag == 0 {
                return;
            }

            let wire_type = tag & 0x07;
            match wire_type {
                0 => {
                    if read_varint(bytes, &mut cursor).is_err() {
                        return;
                    }
                }
                1 => {
                    let Some(next) = cursor.checked_add(8) else {
                        return;
                    };
                    if next > bytes.len() {
                        return;
                    }
                    cursor = next;
                }
                2 => {
                    let Ok(len_varint) = read_varint(bytes, &mut cursor) else {
                        return;
                    };
                    let Ok(len) = usize::try_from(len_varint) else {
                        return;
                    };
                    let Some(end) = cursor.checked_add(len) else {
                        return;
                    };
                    let Some(value) = bytes.get(cursor..end) else {
                        return;
                    };
                    cursor = end;
                    self.collect_length_delimited(value, depth + 1);
                }
                5 => {
                    let Some(next) = cursor.checked_add(4) else {
                        return;
                    };
                    if next > bytes.len() {
                        return;
                    }
                    cursor = next;
                }
                _ => return,
            }
        }
    }

    fn collect_length_delimited(&mut self, bytes: &[u8], depth: usize) {
        if self.push_bytes(bytes) {
            return;
        }

        let Ok(message) = ProtoMessage::decode(bytes) else {
            return;
        };
        self.collect_message(&message, depth);
    }

    fn collect_message(&mut self, message: &ProtoMessage, depth: usize) {
        if depth >= MAX_RECURSION_DEPTH {
            return;
        }

        for field in message.fields() {
            if let ProtoValue::LengthDelimited(bytes) = &field.value {
                self.collect_length_delimited(bytes, depth + 1);
            }
        }
    }
}

fn normalize_utf8_field(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }

    if text
        .chars()
        .any(|ch| ch.is_control() && !ch.is_whitespace())
    {
        return None;
    }

    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, ProtoField};

    #[test]
    fn extracts_only_structural_utf8_fields() -> Result<(), Error> {
        let mut body = ProtoMessage::new(vec![
            ProtoField::string(3, "Visible Text"),
            ProtoField::message(
                4,
                &ProtoMessage::new(vec![ProtoField::string(2, "Nested Text")]),
            )?,
        ])
        .encode()?;
        body.extend_from_slice(b"Raw Printable Noise");

        let mut collector = TextCollector::default();
        collector.collect_field_stream(&body, 0);

        assert_eq!(
            collector.into_strings(),
            vec!["Visible Text".to_owned(), "Nested Text".to_owned()]
        );
        Ok(())
    }
}
