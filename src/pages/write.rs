//! Pages document encoder.

use std::path::Path;

use crate::encode::{
    build_version_history_plist, document_identifier, encode_infra_archives, object_reference,
    properties_plist, synthesize_header, synthesize_header_with_references,
};
use crate::iwa::{IwaArchive, IwaObjectReference};
use crate::package::PackageWriter;
use crate::protobuf::{ProtoField, ProtoMessage};
use crate::Error;

use super::Body;

/// Object IDs used within `Index/Document.iwa`.
const DOCUMENT_ROOT_ID: u64 = 1;
const METADATA_ROOT_ID: u64 = 2;
const OBJECT_CONTAINER_ROOT_ID: u64 = 61;
const DOCUMENT_METADATA_ROOT_ID: u64 = 71;
const ANNOTATION_ROOT_ID: u64 = 80;
const STYLESHEET_ROOT_ID: u64 = 1_002;

/// Type-10001 WP body object ID (stored in `Document.iwa`).
const WP_BODY_ID: u64 = 100;
/// First type-2001 TSWP text object ID (one per text block, allocated upward).
const TSWP_BASE_ID: u64 = 200;

/// Message type of the Pages word-processor body.
const WP_BODY_TYPE: u64 = 10001;
/// Message type of a TSWP text storage object.
const TSWP_TEXT_TYPE: u64 = 2001;

impl Body {
    /// Serializes this Pages body as a minimal `.pages` package.
    ///
    /// The generated package contains `Metadata/`, core `Index/` IWA archives,
    /// and a `Document.iwa` with one type-10001 WP body and one type-2001 TSWP
    /// text storage object per non-empty text section. The package round-trips
    /// through [`Body::from_package`]: `template_name` and `text_fragments` are
    /// preserved; `media_descriptions` are not encoded (no binary image data).
    pub fn to_pages_bytes(&self) -> Result<Vec<u8>, Error> {
        let infra = encode_infra_archives(
            DOCUMENT_ROOT_ID,
            METADATA_ROOT_ID,
            OBJECT_CONTAINER_ROOT_ID,
            DOCUMENT_METADATA_ROOT_ID,
            ANNOTATION_ROOT_ID,
            STYLESHEET_ROOT_ID,
        )?;

        let document_iwa = encode_document_iwa(self)?;

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
            .add_entry("Index/DocumentStylesheet.iwa", infra.document_stylesheet);

        writer.finish()
    }

    /// Writes this Pages body as a `.pages` package to disk.
    pub fn save_pages(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        std::fs::write(path, self.to_pages_bytes()?)?;
        Ok(())
    }
}

/// Encodes `Index/Document.iwa` with a type-10001 WP body object and one
/// type-2001 TSWP text storage object per non-empty text fragment group.
fn encode_document_iwa(body: &Body) -> Result<Vec<u8>, Error> {
    // Collect per-object bytes: (object_id, type_id, payload)
    let mut objects: Vec<(u64, u64, Vec<u8>)> = Vec::new();

    // Encode TSWP text storage objects, one per logical text block.
    // We join all non-empty text fragments with '\n' into a single object so
    // that the decoder's paragraph splitter reconstructs the same fragment list.
    let tswp_id = TSWP_BASE_ID;
    if !body.text_fragments().is_empty() {
        let raw = body.text_fragments().join("\n");
        let tswp_payload = ProtoMessage::new(vec![ProtoField::bytes(3, raw.into_bytes())])
            .encode()?;
        objects.push((tswp_id, TSWP_TEXT_TYPE, tswp_payload));
    }

    // Encode type-10001 WP body object.
    let mut wp_fields: Vec<ProtoField> = Vec::new();
    if let Some(name) = body.template_name() {
        // Field path 1.3 = template name.
        wp_fields.push(ProtoField::message(
            1,
            &ProtoMessage::new(vec![ProtoField::bytes(3, name.as_bytes().to_vec())]),
        )?);
    }
    if !body.text_fragments().is_empty() {
        // Reference to the text storage object.
        wp_fields.push(ProtoField::bytes(2, object_reference(tswp_id)?));
    }
    let wp_payload = ProtoMessage::new(wp_fields).encode()?;
    objects.push((WP_BODY_ID, WP_BODY_TYPE, wp_payload));

    // Assemble the multi-object archive body: each object is its own packet.
    // IwaArchive::encode expects a single contiguous body; we encode a stream
    // by building a PacketStream where each object packet is prepended with its
    // IWA header (root_id, type).  The simplest encoding: place the WP body as
    // the archive root and inline the TSWP objects as additional body packets.
    //
    // IWA multi-object archives encode each object as an independent header+body
    // pair concatenated in the Snappy body. We encode the WP body as the archive
    // root and each TSWP object as a separate sub-object within the same body
    // stream by encoding multiple ProtoMessage blocks in the single flat body.
    //
    // Simpler approach: emit one archive per TSWP object, but that breaks the
    // decoder which scans a single Document.iwa.  Instead, we use the actual IWA
    // multi-body format: root object's header lists a body_hint that covers the
    // WP body payload, then the TSWP objects follow as additional archive entries
    // in the same compressed stream.
    //
    // The current IwaArchive::encode API takes one (header, body) pair.  For
    // multi-object archives we chain multiple IwaArchive::encode calls and
    // concatenate the compressed chunks — they share the Snappy framing and the
    // IWA reader iterates across chunk boundaries.
    encode_multi_object_archive(WP_BODY_ID, WP_BODY_TYPE, &objects)
}

/// Encodes a multi-object IWA archive by concatenating the compressed output of
/// [`IwaArchive::encode`] for each object.
///
/// The IWA reader iterates across chunk boundaries, so concatenating independent
/// compressed archives in one file is equivalent to a single multi-object archive.
fn encode_multi_object_archive(
    root_id: u64,
    _root_type: u64,
    objects: &[(u64, u64, Vec<u8>)],
) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();

    // Non-root objects first so the root (the last entry in `objects`) comes last
    // and acts as the archive root when the reader finds it.
    // Actually, IWA archives designate the root via the header's root_object_id;
    // order in the byte stream doesn't matter for the reader.  We emit the root
    // first for clarity.

    // Find (or encode) the root object.
    let root_bytes = objects
        .iter()
        .find(|(id, _, _)| *id == root_id)
        .map(|(id, typ, payload)| {
            // Build references to all non-root objects so the IWA header declares them.
            let refs: Vec<IwaObjectReference> = objects
                .iter()
                .filter(|(oid, _, _)| *oid != root_id)
                .map(|(oid, _, _)| IwaObjectReference {
                    object_id: Some(*oid),
                    kind_hint: Some(*typ),
                    state_hint: Some(0),
                })
                .collect();
            let header = synthesize_header_with_references(*id, *typ, payload.len(), refs)?;
            IwaArchive::encode(header, payload.clone())
        })
        .transpose()?;

    if let Some(bytes) = root_bytes {
        out.extend_from_slice(&bytes);
    }

    // Emit each non-root object as its own archive chunk.
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
}
