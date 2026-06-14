//! Pages document encoder.

use std::collections::HashMap;
use std::path::Path;

use crate::encode::{
    build_version_history_plist, document_identifier, encode_infra_archives, object_reference,
    properties_plist, synthesize_header, synthesize_header_with_references,
};
use crate::iwa::{IwaArchive, IwaObjectReference};
use crate::protobuf::{ProtoField, ProtoMessage};
use crate::Error;

use super::body::{Paragraph, TextFormatting};
use super::Body;

/// Object IDs used within `Index/Document.iwa`.
const DOCUMENT_ROOT_ID: u64 = 1;
const METADATA_ROOT_ID: u64 = 2;
const OBJECT_CONTAINER_ROOT_ID: u64 = 61;
const DOCUMENT_METADATA_ROOT_ID: u64 = 71;
const ANNOTATION_ROOT_ID: u64 = 80;
const STYLESHEET_ROOT_ID: u64 = 1_002;

/// First type-2001 TSWP text object ID.
const TSWP_BASE_ID: u64 = 200;

/// Type-10001 WP body object ID.
const WP_BODY_ID: u64 = 100;

/// Message type constants for the writer.
const WP_BODY_TYPE: u64 = 10001;
const TSWP_TEXT_TYPE: u64 = 2001;
/// Message type of the `DocumentStylesheet` root.
const STYLESHEET_TYPE: u64 = 401;
/// Message type of a character/paragraph style object.
const STYLE_OBJ_TYPE: u64 = 2022;

/// Base object ID for paragraph style objects in `DocumentStylesheet.iwa`.
const PARA_STYLE_BASE_ID: u64 = 2_000;
/// Base object ID for character style objects in `DocumentStylesheet.iwa`.
const CHAR_STYLE_BASE_ID: u64 = 3_000;

/// Mapping of style names and character formatting to the object IDs used in
/// the encoded package.
struct StyleAssignment {
    /// Paragraph style name → object ID in `DocumentStylesheet.iwa`.
    para: HashMap<String, u64>,
    /// `(TextFormatting, object_id)` pairs for char styles.
    ///
    /// The first entry is always `(default, CHAR_STYLE_BASE_ID)` — the
    /// "reset to normal" style used when transitioning back from non-default
    /// character formatting.
    char_styles: Vec<(TextFormatting, u64)>,
}

impl StyleAssignment {
    fn char_id_for(&self, fmt: &TextFormatting) -> Option<u64> {
        self.char_styles.iter().find(|(f, _)| f == fmt).map(|(_, id)| *id)
    }

    fn para_id_for(&self, name: &str) -> Option<u64> {
        self.para.get(name).copied()
    }
}

fn build_style_assignment(paragraphs: &[Paragraph]) -> StyleAssignment {
    let mut para: HashMap<String, u64> = HashMap::new();
    // "Body" gets the base ID; other styles are allocated above it.
    para.insert("Body".to_owned(), PARA_STYLE_BASE_ID);
    let mut next_para_id = PARA_STYLE_BASE_ID + 1;

    // The "Normal" (reset) char style is always present at the base ID.
    let mut char_styles: Vec<(TextFormatting, u64)> =
        vec![(TextFormatting::default(), CHAR_STYLE_BASE_ID)];
    let mut next_char_id = CHAR_STYLE_BASE_ID + 1;

    for para_obj in paragraphs {
        let name = if para_obj.style_name.is_empty() {
            "Body".to_owned()
        } else {
            para_obj.style_name.clone()
        };
        para.entry(name).or_insert_with(|| {
            let id = next_para_id;
            next_para_id += 1;
            id
        });

        for run in &para_obj.runs {
            if !run.formatting.is_default()
                && !char_styles.iter().any(|(f, _)| f == &run.formatting)
            {
                char_styles.push((run.formatting.clone(), next_char_id));
                next_char_id += 1;
            }
        }
    }

    StyleAssignment { para, char_styles }
}

impl Body {
    /// Serializes this Pages body as a minimal `.pages` package.
    ///
    /// The generated package preserves `template_name` and the full paragraph
    /// list (style names + character formatting). `media_descriptions` are not
    /// encoded (no binary image data).
    pub fn to_pages_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut infra = encode_infra_archives(
            DOCUMENT_ROOT_ID,
            METADATA_ROOT_ID,
            OBJECT_CONTAINER_ROOT_ID,
            DOCUMENT_METADATA_ROOT_ID,
            ANNOTATION_ROOT_ID,
            STYLESHEET_ROOT_ID,
        )?;

        let assignment = build_style_assignment(self.paragraphs());
        let document_iwa = encode_document_iwa(self, &assignment)?;
        infra.document_stylesheet = encode_document_stylesheet_iwa(&assignment)?;

        let mut writer = crate::package::PackageWriter::new();
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
            .add_entry("Index/DocumentStylesheet.iwa", infra.document_stylesheet);

        writer.finish()
    }

    /// Writes this Pages body as a `.pages` package to disk.
    pub fn save_pages(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        std::fs::write(path, self.to_pages_bytes()?)?;
        Ok(())
    }
}

/// Encodes `Index/Document.iwa` with a type-10001 WP body and one type-2001
/// TSWP text storage object whose field 3 carries the raw paragraph text and
/// field 5 carries the paragraph style runs.
fn encode_document_iwa(body: &Body, assignment: &StyleAssignment) -> Result<Vec<u8>, Error> {
    let mut objects: Vec<(u64, u64, Vec<u8>)> = Vec::new();

    let paragraphs = body.paragraphs();
    let tswp_id = TSWP_BASE_ID;

    if !paragraphs.is_empty() {
        let tswp_payload = encode_tswp_payload(paragraphs, assignment)?;
        objects.push((tswp_id, TSWP_TEXT_TYPE, tswp_payload));
    }

    // Encode the type-10001 WP body.
    let mut wp_fields: Vec<ProtoField> = Vec::new();
    if let Some(name) = body.template_name() {
        wp_fields.push(ProtoField::message(
            1,
            &ProtoMessage::new(vec![ProtoField::bytes(3, name.as_bytes().to_vec())]),
        )?);
    }
    if !paragraphs.is_empty() {
        wp_fields.push(ProtoField::bytes(2, object_reference(tswp_id)?));
    }
    let wp_payload = ProtoMessage::new(wp_fields).encode()?;
    objects.push((WP_BODY_ID, WP_BODY_TYPE, wp_payload));

    encode_multi_object_archive(WP_BODY_ID, &objects)
}

/// Encodes a type-2001 TSWP payload: field 3 (raw text), field 5 (para style
/// runs), and field 7 (char style runs when non-trivial).
fn encode_tswp_payload(
    paragraphs: &[Paragraph],
    assignment: &StyleAssignment,
) -> Result<Vec<u8>, Error> {
    let mut raw = String::new();
    let mut para_style_runs: Vec<(usize, u64)> = Vec::new();
    let mut char_style_runs: Vec<(usize, u64)> = Vec::new();

    // Track the last char style ID written so we only emit transitions.
    let mut current_char_id: Option<u64> = None;

    for (para_idx, para) in paragraphs.iter().enumerate() {
        let para_start = raw.len();

        // Paragraph style run at the start of each paragraph.
        let para_style_name =
            if para.style_name.is_empty() { "Body" } else { &para.style_name };
        if let Some(style_id) = assignment.para_id_for(para_style_name) {
            para_style_runs.push((para_start, style_id));
        }

        // Character style runs: emit an entry when formatting changes.
        for run in &para.runs {
            let run_start = raw.len();
            let new_char_id = assignment.char_id_for(&run.formatting);

            // Write a char run entry when the style changes.
            if new_char_id != current_char_id {
                if let Some(id) = new_char_id {
                    char_style_runs.push((run_start, id));
                } else if current_char_id.is_some() {
                    // Transitioning to default — emit the "Normal" reset.
                    char_style_runs.push((run_start, CHAR_STYLE_BASE_ID));
                }
                current_char_id = new_char_id;
            }

            raw.push_str(&run.text);
        }

        if para_idx < paragraphs.len() - 1 {
            raw.push('\n');
        }
    }

    let mut fields = vec![ProtoField::bytes(3, raw.into_bytes())];

    if !para_style_runs.is_empty() {
        fields.push(ProtoField::bytes(5, encode_style_runs_blob(&para_style_runs)?));
    }
    if !char_style_runs.is_empty() {
        fields.push(ProtoField::bytes(7, encode_style_runs_blob(&char_style_runs)?));
    }

    ProtoMessage::new(fields).encode()
}

/// Encodes `Index/DocumentStylesheet.iwa` with a type-401 root carrying the
/// paragraph style registry and type-2022 objects for each char style.
fn encode_document_stylesheet_iwa(assignment: &StyleAssignment) -> Result<Vec<u8>, Error> {
    let mut objects: Vec<(u64, u64, Vec<u8>)> = Vec::new();

    // Encode type-2022 char style objects (including the "Normal" reset entry).
    for (fmt, id) in &assignment.char_styles {
        objects.push((*id, STYLE_OBJ_TYPE, encode_char_style_payload(fmt)?));
    }

    // Build the type-401 root payload with paragraph style registry entries.
    let mut root_fields: Vec<ProtoField> = Vec::new();
    for (name, &id) in &assignment.para {
        let key = format!("text-0-paragraphstyle-{name}");
        let style_ref = object_reference(id)?;
        let entry = ProtoMessage::new(vec![
            ProtoField::string(1, key),
            ProtoField::bytes(2, style_ref),
        ])
        .encode()?;
        root_fields.push(ProtoField::bytes(2, entry));
    }
    let root_payload = ProtoMessage::new(root_fields).encode()?;
    objects.push((STYLESHEET_ROOT_ID, STYLESHEET_TYPE, root_payload));

    encode_multi_object_archive(STYLESHEET_ROOT_ID, &objects)
}

/// Encodes a type-2022 char style object payload.
///
/// Puts character attributes (bold, italic, `font_size_pt`, `font_name`, underline) in
/// the text-attribute sub-message at field 11.
fn encode_char_style_payload(fmt: &TextFormatting) -> Result<Vec<u8>, Error> {
    let mut attr_fields: Vec<ProtoField> = Vec::new();

    if let Some(bold) = fmt.bold {
        attr_fields.push(ProtoField::varint(1, u64::from(bold)));
    }
    if let Some(italic) = fmt.italic {
        attr_fields.push(ProtoField::varint(2, u64::from(italic)));
    }
    if let Some(size) = fmt.font_size_pt {
        attr_fields.push(ProtoField::fixed32(3, size.to_bits()));
    }
    if let Some(ref name) = fmt.font_name {
        attr_fields.push(ProtoField::string(5, name.clone()));
    }
    if let Some(underline) = fmt.underline {
        attr_fields.push(ProtoField::varint(13, u64::from(underline)));
    }

    if attr_fields.is_empty() {
        return Ok(Vec::new());
    }

    ProtoMessage::new(vec![ProtoField::message(11, &ProtoMessage::new(attr_fields))?]).encode()
}

/// Encodes a style-runs blob (used for type-2001 field 5 and field 7).
///
/// Wire structure: outer blob is a sub-message with repeated field-1 entries;
/// each entry encodes `{field 1: byte_offset, field 2: {field 1: style_id}}`.
fn encode_style_runs_blob(runs: &[(usize, u64)]) -> Result<Vec<u8>, Error> {
    let run_fields: Vec<ProtoField> = runs
        .iter()
        .map(|(offset, style_id)| {
            let style_ref =
                ProtoMessage::new(vec![ProtoField::varint(1, *style_id)]).encode()?;
            let run = ProtoMessage::new(vec![
                ProtoField::varint(1, *offset as u64),
                ProtoField::bytes(2, style_ref),
            ])
            .encode()?;
            Ok(ProtoField::bytes(1, run))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    ProtoMessage::new(run_fields).encode()
}

/// Encodes a multi-object IWA archive.
///
/// The root object's header lists all non-root objects as cross-references.
/// Non-root objects follow as independent archive chunks.
fn encode_multi_object_archive(
    root_id: u64,
    objects: &[(u64, u64, Vec<u8>)],
) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();

    let root_bytes = objects
        .iter()
        .find(|(id, _, _)| *id == root_id)
        .map(|(id, typ, payload)| {
            let refs: Vec<IwaObjectReference> = objects
                .iter()
                .filter(|(oid, _, _)| *oid != root_id)
                .map(|(oid, obj_typ, _)| IwaObjectReference {
                    object_id: Some(*oid),
                    kind_hint: Some(*obj_typ),
                    state_hint: Some(0),
                })
                .collect();
            let header =
                synthesize_header_with_references(*id, *typ, payload.len(), refs)?;
            IwaArchive::encode(header, payload.clone())
        })
        .transpose()?;

    if let Some(bytes) = root_bytes {
        out.extend_from_slice(&bytes);
    }

    for (id, typ, payload) in objects.iter().filter(|(id, _, _)| *id != root_id) {
        let header = synthesize_header(*id, *typ, payload.len())?;
        out.extend_from_slice(&IwaArchive::encode(header, payload.clone())?);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages;
    use crate::pages::body::{Paragraph, TextFormatting, TextRun};

    #[test]
    fn body_round_trips_text_fragments() -> Result<(), Error> {
        let mut body = Body::default();
        body.set_text_fragments(vec![
            "Hello".to_owned(),
            "World".to_owned(),
            "Goodbye".to_owned(),
        ]);
        let bytes = body.to_pages_bytes()?;
        let doc = pages::Document::from_bytes(bytes)?;
        let decoded = doc.document()?;
        assert_eq!(decoded.text_fragments(), body.text_fragments());
        Ok(())
    }

    #[test]
    fn body_round_trips_template_name() -> Result<(), Error> {
        let mut body = Body::default();
        body.set_template_name(Some("04B_Term_Paper".to_owned()));
        body.set_text_fragments(vec!["Some text".to_owned()]);
        let bytes = body.to_pages_bytes()?;
        let doc = pages::Document::from_bytes(bytes)?;
        let decoded = doc.document()?;
        assert_eq!(decoded.template_name(), Some("04B_Term_Paper"));
        Ok(())
    }

    #[test]
    fn body_saves_to_pages_file() -> Result<(), Error> {
        let path =
            std::env::temp_dir().join(format!("iwork-generated-{}.pages", std::process::id()));
        let mut body = Body::default();
        body.set_text_fragments(vec!["Saved".to_owned()]);
        body.save_pages(&path)?;
        let doc = pages::Document::open(&path)?;
        let decoded = doc.document()?;
        assert!(decoded.text_fragments().contains(&"Saved".to_owned()));
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn body_round_trips_paragraph_style_names() -> Result<(), Error> {
        let mut body = Body::default();
        body.set_paragraphs(vec![
            Paragraph {
                style_name: "Title".to_owned(),
                runs: vec![TextRun {
                    text: "My Novel".to_owned(),
                    formatting: TextFormatting::default(),
                }],
            },
            Paragraph {
                style_name: "Heading".to_owned(),
                runs: vec![TextRun {
                    text: "Chapter One".to_owned(),
                    formatting: TextFormatting::default(),
                }],
            },
            Paragraph {
                style_name: "Body".to_owned(),
                runs: vec![TextRun {
                    text: "It was a dark and stormy night.".to_owned(),
                    formatting: TextFormatting::default(),
                }],
            },
        ]);
        assert_eq!(body.title(), Some("My Novel"));
        assert_eq!(body.headings(), &["Chapter One"]);
        assert_eq!(body.text_fragments(), &["It was a dark and stormy night."]);

        let bytes = body.to_pages_bytes()?;
        let doc = pages::Document::from_bytes(bytes)?;
        let decoded = doc.document()?;

        assert_eq!(decoded.title(), Some("My Novel"), "title should round-trip");
        assert_eq!(decoded.headings(), &["Chapter One"], "headings should round-trip");
        assert_eq!(
            decoded.text_fragments(),
            &["It was a dark and stormy night."],
            "text_fragments should round-trip"
        );

        let paras = decoded.paragraphs();
        assert_eq!(paras.len(), 3, "should have 3 paragraphs");
        assert_eq!(paras[0].style_name, "Title");
        assert_eq!(paras[0].text(), "My Novel");
        assert_eq!(paras[1].style_name, "Heading");
        assert_eq!(paras[2].style_name, "Body");

        Ok(())
    }

    #[test]
    fn body_round_trips_character_formatting() -> Result<(), Error> {
        let mut body = Body::default();
        body.set_paragraphs(vec![Paragraph {
            style_name: "Body".to_owned(),
            runs: vec![
                TextRun {
                    text: "plain ".to_owned(),
                    formatting: TextFormatting::default(),
                },
                TextRun {
                    text: "bold".to_owned(),
                    formatting: TextFormatting { bold: Some(true), ..Default::default() },
                },
                TextRun {
                    text: " plain".to_owned(),
                    formatting: TextFormatting::default(),
                },
            ],
        }]);

        let bytes = body.to_pages_bytes()?;
        let doc = pages::Document::from_bytes(bytes)?;
        let decoded = doc.document()?;

        let paras = decoded.paragraphs();
        assert_eq!(paras.len(), 1);
        let runs = &paras[0].runs;

        // Find the bold run.
        let bold_run = runs.iter().find(|r| r.formatting.bold == Some(true));
        assert!(bold_run.is_some(), "should have a bold run; runs: {runs:?}");
        assert_eq!(bold_run.unwrap().text, "bold");

        Ok(())
    }
}
