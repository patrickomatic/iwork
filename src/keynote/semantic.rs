use crate::iwa::IwaArchive;
use crate::iwa_text::extract_utf8_fields;
use crate::{Error, Package};
use std::path::Path;

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
            .filter(|entry| {
                Path::new(&entry.path)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("iwa"))
            })
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

/// UTF-8 string fields decoded from a Keynote slide archive.
///
/// This walks the decoded IWA protobuf fields for each slide-related archive.
/// It does not classify layout names, titles, media descriptions, presenter
/// notes, or animations until those Keynote object fields are decoded
/// explicitly.
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
        let fragments = extract_utf8_fields(archive);

        Self {
            is_template: path.contains("TemplateSlide"),
            layout_name: None,
            media_descriptions: Vec::new(),
            path,
            text_fragments: fragments,
            title: None,
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
