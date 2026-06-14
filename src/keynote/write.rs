//! Keynote presentation encoder.

use std::path::Path;

use crate::encode::{
    build_version_history_plist, document_identifier, encode_infra_archives, properties_plist,
    synthesize_header, synthesize_header_with_references,
};
use crate::iwa::{IwaArchive, IwaObjectReference};
use crate::package::PackageWriter;
use crate::protobuf::{ProtoField, ProtoMessage};
use crate::Error;

use super::presentation::{Presentation, Slide};

/// Package-global object IDs used across `.key` archives.
const DOCUMENT_ROOT_ID: u64 = 1;
const METADATA_ROOT_ID: u64 = 2;
const OBJECT_CONTAINER_ROOT_ID: u64 = 61;
const DOCUMENT_METADATA_ROOT_ID: u64 = 71;
const ANNOTATION_ROOT_ID: u64 = 80;
const STYLESHEET_ROOT_ID: u64 = 1_002;

/// ID of the type-10 theme object in `Document.iwa`.
const THEME_ID: u64 = 100;

/// Message type of the Keynote theme descriptor.
const THEME_TYPE: u64 = 10;

/// Message type of a Keynote drawable / placeholder object.
const DRAWABLE_TYPE: u64 = 7;
/// Message type of a TSWP text storage object.
const TSWP_TEXT_TYPE: u64 = 2001;

/// `field 2` values for drawable placeholder kinds.
const PLACEHOLDER_TITLE: u64 = 2;
const PLACEHOLDER_BODY: u64 = 3;

/// Within each per-slide archive, use these local object IDs.
///
/// They are local because each slide is its own `.iwa` file; the decoder never
/// cross-resolves IDs between slide archives.
const SLIDE_TITLE_DRAWABLE_ID: u64 = 1;
const SLIDE_BODY_DRAWABLE_ID: u64 = 2;
const SLIDE_TITLE_TEXT_ID: u64 = 10;
const SLIDE_BODY_TEXT_ID: u64 = 11;

impl Presentation {
    /// Serializes this presentation as a minimal `.key` package.
    ///
    /// The generated package contains `Metadata/`, core `Index/` IWA archives,
    /// `Index/Document.iwa` with the theme object, one `Index/Slide-N.iwa` per
    /// non-template slide, and one `Index/TemplateSlide-1.iwa` stub (Keynote
    /// requires at least one template slide). The package round-trips through
    /// [`Presentation::from_package`]: `theme_name`, slide `title`, and slide
    /// `text_fragments` are preserved. `media_descriptions` are not encoded.
    pub fn to_keynote_bytes(&self) -> Result<Vec<u8>, Error> {
        let infra = encode_infra_archives(
            DOCUMENT_ROOT_ID,
            METADATA_ROOT_ID,
            OBJECT_CONTAINER_ROOT_ID,
            DOCUMENT_METADATA_ROOT_ID,
            ANNOTATION_ROOT_ID,
            STYLESHEET_ROOT_ID,
        )?;

        let document_iwa = encode_document_iwa(self.theme_name())?;

        // Template slide stub — Keynote expects at least one.
        let template_slide_iwa = encode_empty_slide_archive()?;

        let mut writer = PackageWriter::new();
        writer
            .add_entry("Metadata/Properties.plist", properties_plist())
            .add_entry("Metadata/DocumentIdentifier", document_identifier())
            .add_entry(
                "Metadata/BuildVersionHistory.plist",
                build_version_history_plist(),
            )
            .add_entry("Index/Document.iwa", document_iwa)
            .add_entry("Index/DocumentMetadata.iwa", infra.document_metadata)
            .add_entry("Index/Metadata.iwa", infra.metadata)
            .add_entry("Index/ObjectContainer.iwa", infra.object_container)
            .add_entry(
                "Index/AnnotationAuthorStorage.iwa",
                infra.annotation_author_storage,
            )
            .add_entry("Index/DocumentStylesheet.iwa", infra.document_stylesheet)
            .add_entry("Index/TemplateSlide-1.iwa", template_slide_iwa);

        let real_slides: Vec<&Slide> =
            self.slides().iter().filter(|s| !s.is_template()).collect();
        for (n, slide) in real_slides.iter().enumerate() {
            let slide_iwa = encode_slide_iwa(slide)?;
            writer.add_entry(format!("Index/Slide-{}.iwa", n + 1), slide_iwa);
        }

        writer.finish()
    }

    /// Writes this presentation as a `.key` package to disk.
    pub fn save_keynote(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        std::fs::write(path, self.to_keynote_bytes()?)?;
        Ok(())
    }
}

/// Encodes `Index/Document.iwa` with a single type-10 theme object.
///
/// Field path for the theme name: `field 1` (nested) → `field 3` (UTF-8).
fn encode_document_iwa(theme_name: Option<&str>) -> Result<Vec<u8>, Error> {
    let mut inner_fields: Vec<ProtoField> = Vec::new();
    if let Some(name) = theme_name {
        inner_fields.push(ProtoField::bytes(3, name.as_bytes().to_vec()));
    }
    let theme_payload = ProtoMessage::new(vec![ProtoField::message(
        1,
        &ProtoMessage::new(inner_fields),
    )?])
    .encode()?;

    let header = synthesize_header_with_references(
        THEME_ID,
        THEME_TYPE,
        theme_payload.len(),
        vec![IwaObjectReference {
            object_id: Some(DOCUMENT_ROOT_ID),
            kind_hint: Some(1),
            state_hint: Some(0),
        }],
    )?;
    IwaArchive::encode(header, theme_payload)
}

/// Encodes a single real (non-template) `Index/Slide-N.iwa`.
///
/// The archive contains:
/// - type-7 title drawable (id=1) referencing the title type-2001 (id=10)
/// - type-7 body drawable (id=2) referencing the body type-2001 (id=11)
/// - type-2001 title text (id=10): slide title
/// - type-2001 body text (id=11): remaining `text_fragments` joined with `\n`
///
/// When there is no title the title drawable still gets an empty type-2001 so
/// the decoder's `decode_slide_title` finds it and returns `None` (empty text
/// is filtered out). When there are no body fragments the body objects are
/// omitted to keep the archive minimal.
fn encode_slide_iwa(slide: &Slide) -> Result<Vec<u8>, Error> {
    let mut chunks: Vec<Vec<u8>> = Vec::new();

    // --- title text object ---
    let title_text = slide.title().unwrap_or("");
    let title_payload =
        ProtoMessage::new(vec![ProtoField::bytes(3, title_text.as_bytes().to_vec())])
            .encode()?;
    let title_header = synthesize_header(SLIDE_TITLE_TEXT_ID, TSWP_TEXT_TYPE, title_payload.len())?;
    chunks.push(IwaArchive::encode(title_header, title_payload)?);

    // --- title drawable (field 1.4.1 → SLIDE_TITLE_TEXT_ID, field 2 = 2) ---
    let title_drawable_payload = encode_drawable_payload(SLIDE_TITLE_TEXT_ID, PLACEHOLDER_TITLE)?;
    let title_drawable_header =
        synthesize_header(SLIDE_TITLE_DRAWABLE_ID, DRAWABLE_TYPE, title_drawable_payload.len())?;
    chunks.push(IwaArchive::encode(title_drawable_header, title_drawable_payload)?);

    // --- body text and drawable ---
    let body_fragments: Vec<&str> = slide
        .text_fragments()
        .iter()
        .filter(|f| slide.title().is_none_or(|t| *f != t))
        .map(String::as_str)
        .collect();

    if !body_fragments.is_empty() {
        let body_text = body_fragments.join("\n");
        let body_payload =
            ProtoMessage::new(vec![ProtoField::bytes(3, body_text.into_bytes())]).encode()?;
        let body_header =
            synthesize_header(SLIDE_BODY_TEXT_ID, TSWP_TEXT_TYPE, body_payload.len())?;
        chunks.push(IwaArchive::encode(body_header, body_payload)?);

        let body_drawable_payload =
            encode_drawable_payload(SLIDE_BODY_TEXT_ID, PLACEHOLDER_BODY)?;
        let body_drawable_header = synthesize_header(
            SLIDE_BODY_DRAWABLE_ID,
            DRAWABLE_TYPE,
            body_drawable_payload.len(),
        )?;
        chunks.push(IwaArchive::encode(body_drawable_header, body_drawable_payload)?);
    }

    Ok(chunks.into_iter().flatten().collect())
}

/// Encodes a type-7 drawable payload with:
/// - `field 2` = `placeholder_kind`
/// - `field 1.4.1` = `text_object_id` as a varint
///
/// The decoder chain is:
/// `msg.field(1).as_bytes → decode → field(4).as_bytes → decode → field(1).as_varint()`
fn encode_drawable_payload(text_object_id: u64, placeholder_kind: u64) -> Result<Vec<u8>, Error> {
    ProtoMessage::new(vec![
        ProtoField::message(
            1,
            &ProtoMessage::new(vec![ProtoField::message(
                4,
                &ProtoMessage::new(vec![ProtoField::varint(1, text_object_id)]),
            )?]),
        )?,
        ProtoField::varint(2, placeholder_kind),
    ])
    .encode()
}

/// Encodes a minimal empty template slide archive (no objects, empty body).
fn encode_empty_slide_archive() -> Result<Vec<u8>, Error> {
    let header = synthesize_header(1, DRAWABLE_TYPE, 0)?;
    IwaArchive::encode(header, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keynote;

    fn make_presentation(theme: &str, slides: Vec<(&str, Vec<&str>)>) -> Presentation {
        let mut p = Presentation::default();
        p.set_theme_name(Some(theme.to_owned()));
        for (title, frags) in slides {
            let s = Slide::new_real(title, frags.into_iter().map(ToOwned::to_owned).collect());
            p.add_slide(s);
        }
        p
    }

    #[test]
    fn presentation_round_trips_theme_name() -> Result<(), Error> {
        let p = make_presentation("Blueprint", vec![]);
        let bytes = p.to_keynote_bytes()?;
        let doc = keynote::Document::from_bytes(bytes)?;
        let decoded = doc.presentation()?;
        assert_eq!(decoded.theme_name(), Some("Blueprint"));
        Ok(())
    }

    #[test]
    fn presentation_round_trips_slide_title() -> Result<(), Error> {
        let p = make_presentation(
            "Basic",
            vec![("My Title", vec!["My Title", "Some body text"])],
        );
        let bytes = p.to_keynote_bytes()?;
        let doc = keynote::Document::from_bytes(bytes)?;
        let decoded = doc.presentation()?;
        let real_slides: Vec<_> = decoded.slides().iter().filter(|s| !s.is_template()).collect();
        assert_eq!(real_slides.len(), 1);
        assert_eq!(real_slides[0].title(), Some("My Title"));
        assert!(real_slides[0].text_fragments().contains(&"Some body text".to_owned()));
        Ok(())
    }

    #[test]
    fn presentation_saves_to_keynote_file() -> Result<(), Error> {
        let path =
            std::env::temp_dir().join(format!("iwork-generated-{}.key", std::process::id()));
        let p = make_presentation("Test", vec![("Hello", vec!["Hello"])]);
        p.save_keynote(&path)?;
        let doc = keynote::Document::open(&path)?;
        let decoded = doc.presentation()?;
        assert_eq!(decoded.theme_name(), Some("Test"));
        std::fs::remove_file(path)?;
        Ok(())
    }
}
