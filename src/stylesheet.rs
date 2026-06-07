use std::collections::BTreeSet;

use crate::iwa::IwaArchive;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StylesheetCatalog {
    pub referenced_object_ids: Vec<u64>,
    pub identifiers: Vec<String>,
    pub font_names: Vec<String>,
    pub style_names: Vec<String>,
}

impl StylesheetCatalog {
    pub fn from_archive(archive: &IwaArchive) -> Self {
        let referenced_object_ids = archive.leading_object_references();
        let strings = archive.ascii_strings(8);

        let mut identifiers = BTreeSet::new();
        let mut font_names = BTreeSet::new();
        let mut style_names = BTreeSet::new();

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
        }
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
