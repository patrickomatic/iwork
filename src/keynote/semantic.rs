use crate::iwa::IwaArchive;
use crate::{Error, Package};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticPresentation {
    slides: Vec<SemanticSlide>,
}

impl SemanticPresentation {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let mut slides = package
            .entries()
            .iter()
            .filter(|entry| entry.path.starts_with("Index/"))
            .filter(|entry| entry.path.contains("Slide"))
            .filter(|entry| entry.path.ends_with(".iwa"))
            .map(|entry| {
                let archive = IwaArchive::decode(package.entry_bytes(&entry.path)?)?;
                Ok(SemanticSlide::from_archive(entry.path.clone(), &archive))
            })
            .collect::<Result<Vec<_>, Error>>()?;

        slides.sort_by(|left, right| left.path.cmp(&right.path));
        slides.retain(|slide| {
            slide.title.is_some()
                || slide.layout_name.is_some()
                || !slide.text_fragments.is_empty()
                || !slide.media_descriptions.is_empty()
        });

        Ok(Self { slides })
    }

    pub fn slides(&self) -> &[SemanticSlide] {
        &self.slides
    }
}

/// Best-effort semantic content extracted from a Keynote slide archive.
///
/// Current limitations:
/// - slide content is recovered from printable archive runs rather than a full
///   decode of Keynote's slide object graph
/// - template slides and live slides are both surfaced because both can contain
///   meaningful placeholder or content text in current fixtures
/// - text ordering is approximate and presenter notes / animation structure are
///   not reconstructed yet
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticSlide {
    path: String,
    is_template: bool,
    layout_name: Option<String>,
    title: Option<String>,
    text_fragments: Vec<String>,
    media_descriptions: Vec<String>,
}

impl SemanticSlide {
    fn from_archive(path: String, archive: &IwaArchive) -> Self {
        let strings = archive.ascii_strings(4);
        let fragments = extract_text_fragments(strings);
        let layout_name = fragments
            .iter()
            .find(|fragment| is_layout_name(fragment))
            .cloned();
        let title = fragments
            .iter()
            .find(|fragment| is_title_candidate(fragment))
            .cloned();
        let media_descriptions = fragments
            .iter()
            .filter(|fragment| is_media_description(fragment))
            .cloned()
            .collect();

        Self {
            is_template: path.contains("TemplateSlide"),
            layout_name,
            media_descriptions,
            path,
            text_fragments: fragments,
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

fn extract_text_fragments(strings: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for string in strings {
        let normalized = normalize_fragment(&string);
        if is_semantic_text(&normalized) && !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

fn normalize_fragment(fragment: &str) -> String {
    let normalized = fragment
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '*' | ',' | ';' | ':' | '@' | '<' | '>'))
        .replace('\u{fffc}', "")
        .replace('\u{2019}', "'")
        .replace('\u{2014}', "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    strip_media_wrapper(&normalized)
}

fn is_semantic_text(fragment: &str) -> bool {
    if fragment.len() < 4 || fragment.len() > 160 {
        return false;
    }
    if !fragment.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    if looks_like_noise(fragment) {
        return false;
    }

    let keep_chars = fragment
        .chars()
        .filter(|&ch| {
            ch.is_ascii_alphanumeric()
                || matches!(ch, ' ' | '.' | ',' | '\'' | '-' | '&' | '%' | '!' | '?')
        })
        .count();
    keep_chars * 100 >= fragment.chars().count() * 75
}

fn looks_like_noise(fragment: &str) -> bool {
    const EXACT_NOISE: &[&str] = &[
        "en_US",
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
        "1st quarter",
        "2nd quarter",
        "3rd quarter",
        "4th quarter",
        "Before Christ",
        "Anno Domini",
        "M/d/yy",
        "MMM d, y",
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
        "none",
        "Transition",
        "Default Camera",
        "Text",
        "Text-0",
        "Text-1",
        "Text-2",
        "Text-3",
        "Text-4",
        "Text-5",
        "Media",
        "Media-2",
        "Media-3",
        "KNLiveVideos",
        "Body Level One",
        "NBody Level One",
        "Title Text",
    ];

    if EXACT_NOISE.contains(&fragment) {
        return true;
    }
    if fragment.contains("Application/")
        || fragment.contains(".jpeg")
        || fragment.contains(".jpg")
        || fragment.contains(".png")
        || fragment.contains("LiveVideos")
        || fragment.contains("paragraphstyle")
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

fn is_layout_name(fragment: &str) -> bool {
    matches!(
        fragment,
        "Title"
            | "Title Only"
            | "Title & Photo"
            | "Title & Photo Alt"
            | "Title & Bullets"
            | "Title, Bullets & Photo"
            | "Title, Bullets & Live Video Small"
            | "Title, Bullets & Live Video Large"
            | "Title - Center"
            | "Title - Top"
    )
}

fn is_title_candidate(fragment: &str) -> bool {
    matches!(
        fragment,
        "Slide Title" | "Slide Subtitle" | "Blueprint" | "Parchment"
    ) || is_layout_name(fragment)
}

fn is_media_description(fragment: &str) -> bool {
    fragment.len() >= 20
        && fragment.contains(' ')
        && fragment
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
        && !is_layout_name(fragment)
        && !matches!(
            fragment,
            "Presentation Subtitle" | "Slide Title" | "Slide Subtitle"
        )
}

fn strip_media_wrapper(fragment: &str) -> String {
    if fragment.ends_with('R') {
        let chars: Vec<char> = fragment.chars().collect();
        if chars.len() > 4
            && chars[0].is_ascii_uppercase()
            && !chars[1].is_ascii_lowercase()
            && chars[2].is_ascii_uppercase()
        {
            let candidate: String = chars[2..chars.len() - 1].iter().collect();
            if candidate.contains(' ') {
                return candidate;
            }
        }
    }
    fragment.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_known_keynote_noise() {
        assert!(looks_like_noise("Transition"));
        assert!(looks_like_noise("Default Camera"));
        assert!(looks_like_noise("Application/Blueprint/Wide"));
        assert!(!looks_like_noise(
            "Pyramids of Giza silhouetted against an orange sunset"
        ));
    }

    #[test]
    fn classifies_layouts_titles_and_media() {
        assert!(is_layout_name("Title & Photo"));
        assert!(is_title_candidate("Slide Title"));
        assert!(is_title_candidate("Blueprint"));
        assert!(is_media_description(
            "Front of a modern house lit up at night with wide stairs in the front"
        ));
    }
}
