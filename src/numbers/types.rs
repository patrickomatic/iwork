//! Apple iWork message type identifiers seen in Numbers packages.
//!
//! Every `.iwa` archive (and every object inside one) declares a numeric
//! message type in its `ArchiveInfo`. The mappings below are grounded in
//! *structural* evidence from the package itself: a top-level archive whose ZIP
//! entry is named `Index/<Role>.iwa` (or `Index/Tables/<Role>-*.iwa`) carries a
//! root object whose type identifier is the value listed here. That filename →
//! type correspondence holds across every Numbers fixture and does not depend on
//! the document's contents.
//!
//! Child object types that only appear *inside* a composite archive (for
//! example the sheet and table objects packed into `Index/Document.iwa`) are
//! intentionally omitted until their identity is confirmed structurally rather
//! than guessed from a single document.

/// Returns the role name for a known top-level Numbers archive type identifier.
///
/// The name describes the archive's role as evidenced by its ZIP entry name;
/// it is not Apple's internal protobuf message name. Returns `None` for type
/// identifiers that have not been grounded in structural evidence yet.
pub fn message_type_name(message_type: u64) -> Option<&'static str> {
    let name = match message_type {
        1 => "Document",
        210 => "ViewState",
        213 => "AnnotationAuthorStorage",
        401 => "DocumentStylesheet",
        4000 => "CalculationEngine",
        6002 => "Tile",
        6005 => "DataList",
        6006 => "HeaderStorageBucket",
        11006 => "Metadata",
        11008 => "ObjectContainer",
        11011 => "DocumentMetadata",
        _ => return None,
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::message_type_name;

    #[test]
    fn names_known_archive_types() {
        assert_eq!(message_type_name(6002), Some("Tile"));
        assert_eq!(message_type_name(6005), Some("DataList"));
        assert_eq!(message_type_name(1), Some("Document"));
    }

    #[test]
    fn returns_none_for_unknown_types() {
        assert_eq!(message_type_name(0), None);
        assert_eq!(message_type_name(999_999), None);
    }
}
