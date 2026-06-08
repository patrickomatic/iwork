use crate::{Error, Package};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";

/// Best-effort semantic text extracted from a Pages document.
///
/// Current limitations:
/// - text is recovered from printable archive runs, not a fully decoded Pages
///   paragraph/text-run object graph
/// - titles may be `None` when the visible title is split across archive
///   fragments, as in `modern_novel.pages`
/// - body prose can still contain partial or template-derived fragments because
///   the underlying archive encoding interleaves content with formatting data
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticDocument {
    title: Option<String>,
    headings: Vec<String>,
    text_fragments: Vec<String>,
}

impl SemanticDocument {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let bytes = package.entry_bytes(DOCUMENT_ENTRY)?;
        let text_fragments = extract_text_fragments(bytes);
        let title = detect_title(&text_fragments);
        let headings = detect_headings(&text_fragments, title.as_deref());

        Ok(Self {
            title,
            headings,
            text_fragments,
        })
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
}

fn extract_text_fragments(bytes: &[u8]) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut current = Vec::new();

    for &byte in bytes {
        if byte.is_ascii_graphic() || byte == b' ' {
            current.push(byte);
        } else {
            push_fragment(&mut fragments, &mut current);
        }
    }
    push_fragment(&mut fragments, &mut current);

    let mut deduped = Vec::new();
    for fragment in fragments {
        if deduped.last() != Some(&fragment) {
            deduped.push(fragment);
        }
    }
    deduped
}

fn push_fragment(out: &mut Vec<String>, current: &mut Vec<u8>) {
    if current.len() < 4 {
        current.clear();
        return;
    }

    let fragment = normalize_fragment(&String::from_utf8_lossy(current));
    current.clear();

    if is_semantic_text(&fragment) {
        out.push(fragment);
    }
}

fn normalize_fragment(fragment: &str) -> String {
    let normalized = fragment
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '*' | ',' | ';' | ':' | '@'))
        .replace('\u{fffc}', "")
        .replace('\u{2019}', "'")
        .replace('\u{2014}', "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    normalize_heading_fragment(&normalized)
}

fn is_semantic_text(fragment: &str) -> bool {
    if fragment.len() < 4 || fragment.len() > 120 {
        return false;
    }
    if fragment.is_ascii() && !fragment.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    if looks_like_noise(fragment) {
        return false;
    }
    if !fragment.contains(' ') && !is_likely_heading(fragment) {
        return false;
    }

    let keep_chars = fragment
        .chars()
        .filter(|&ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '.' | ',' | '\'' | '-' | '!' | '?')
        })
        .count();
    keep_chars * 100 >= fragment.chars().count() * 75
}

fn looks_like_noise(fragment: &str) -> bool {
    const EXACT_NOISE: &[&str] = &[
        "en_US",
        "en_USP",
        "gregorian",
        "latn",
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Before Christ",
        "Anno Domini",
        "1st quarter",
        "2nd quarter",
        "3rd quarter",
        "4th quarter",
        "M/d/yy",
        "MMM d, y",
        "MMMM d, y",
        "EEEE, MMMM d, y",
        "EEEE, M",
        "h:mm",
        "h:mm:ss",
        "a zzzz",
        "#,##0.###",
        "#,##0%",
        "#,##0.00",
        "FCFA",
        "CFPF",
        "decimal",
        "edit",
        "Format",
        "Drag",
        "Blank",
        "Standard TOC",
        "1234 Main Street",
        "Anytown, State ZIP",
        "www.example.com!@",
    ];

    if EXACT_NOISE.contains(&fragment) {
        return true;
    }
    if fragment.contains("Application/")
        || fragment.contains(".com")
        || fragment.contains("iCloud.com")
        || fragment.contains("Brother_HL_")
        || fragment.contains("Term_Paper")
        || fragment.contains("Novel_Modern")
        || looks_like_uuid(fragment)
        || looks_like_format_code(fragment)
    {
        return true;
    }
    if fragment
        .chars()
        .filter(|ch| ch.is_ascii_punctuation())
        .count()
        > fragment.len() / 3
    {
        return true;
    }
    false
}

fn looks_like_uuid(fragment: &str) -> bool {
    fragment.len() == 36
        && fragment.chars().enumerate().all(|(idx, ch)| match idx {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        })
}

fn looks_like_format_code(fragment: &str) -> bool {
    fragment.chars().all(|ch| {
        ch.is_ascii_uppercase()
            || ch.is_ascii_digit()
            || matches!(ch, '#' | '%' | '.' | '/' | ':' | ',' | '-')
    })
}

fn detect_title(fragments: &[String]) -> Option<String> {
    fragments
        .iter()
        .find(|fragment| is_likely_title(fragment))
        .cloned()
}

fn is_likely_title(fragment: &str) -> bool {
    if fragment.len() < 8 || fragment.len() > 80 {
        return false;
    }
    if matches!(
        fragment,
        "Author Name" | "Subheading" | "Prologue" | "Fall 2023"
    ) {
        return false;
    }
    if fragment.starts_with("Chapter ") {
        return false;
    }

    let words: Vec<_> = fragment.split_whitespace().collect();
    if words.len() < 3 || words.len() > 8 {
        return false;
    }
    if words[0].eq_ignore_ascii_case("of") {
        return false;
    }
    if words[0]
        .chars()
        .next()
        .is_none_or(|ch| !ch.is_ascii_uppercase() && !ch.is_ascii_digit())
    {
        return false;
    }

    words.iter().all(|word| {
        word.chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
            || matches!(
                *word,
                "of" | "the" | "and" | "to" | "for" | "in" | "on" | "with"
            )
    })
}

fn detect_headings(fragments: &[String], title: Option<&str>) -> Vec<String> {
    let mut headings = Vec::new();

    for fragment in fragments {
        if Some(fragment.as_str()) == title {
            continue;
        }
        if is_likely_heading(fragment) && !headings.contains(fragment) {
            headings.push(fragment.clone());
        }
    }

    headings
}

fn is_likely_heading(fragment: &str) -> bool {
    if fragment.len() < 4 || fragment.len() > 60 {
        return false;
    }
    if fragment.ends_with('.') {
        return false;
    }
    if matches!(fragment, "Author Name" | "Fall 2023") {
        return false;
    }
    if fragment.starts_with("Chapter ")
        || matches!(fragment, "Prologue" | "Introduction" | "Subheading")
    {
        return true;
    }
    false
}

fn normalize_heading_fragment(fragment: &str) -> String {
    if let Some(rest) = fragment.strip_prefix("Chapter ") {
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return format!("Chapter {digits}");
        }
    }
    fragment.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_known_formatting_noise() {
        assert!(looks_like_noise("January"));
        assert!(looks_like_noise("Application/11B_Novel_Modern/Traditional"));
        assert!(looks_like_noise("Brother_HL_L2340D_series"));
        assert!(!looks_like_noise("Story of the Night Sky"));
    }

    #[test]
    fn identifies_titles_and_headings() {
        let fragments = vec![
            "Story of the Night Sky".to_owned(),
            "Author Name".to_owned(),
            "Prologue".to_owned(),
            "Chapter 1".to_owned(),
            "Subheading".to_owned(),
        ];

        assert_eq!(
            detect_title(&fragments).as_deref(),
            Some("Story of the Night Sky")
        );
        assert_eq!(
            detect_headings(&fragments, Some("Story of the Night Sky")),
            vec!["Prologue", "Chapter 1", "Subheading"]
        );
    }
}
