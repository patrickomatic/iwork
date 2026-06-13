//! Apple iWork message type identifiers seen in Numbers packages.
//!
//! Every `.iwa` archive (and every object inside one) declares a numeric
//! message type in its `ArchiveInfo`. The mappings below are grounded in
//! *structural* evidence, by one of two methods:
//!
//! - **Filename evidence** (top-level archives): an archive whose ZIP entry is
//!   named `Index/<Role>.iwa` (or `Index/Tables/<Role>-*.iwa`) carries a root
//!   object whose type identifier is the value listed here. This correspondence
//!   holds across every fixture and does not depend on document content.
//! - **Reference-graph evidence** (in-stream objects): an object's identity is
//!   fixed by its position in the cross-object reference graph — which object
//!   types reference it and which it references — together with a count that
//!   tracks document structure (sheets, tables) rather than content. Object
//!   identifiers are large unique integers, so a payload varint equal to another
//!   object's identifier is a reliable reference edge.
//!
//! Reference-graph grounding for the table chain
//! (`Sheet → TableInfo → TableModel → Tile + DataList + HeaderStorageBucket`):
//!
//! - `2` (Sheet): referenced by the `Document` root; its count equals the sheet
//!   count; it references the per-sheet `TableInfo` objects.
//! - `6000` (TableInfo): each references exactly one `TableModel`; the count of
//!   `TableInfo` and `TableModel` objects are equal (one wraps one).
//! - `6001` (TableModel): references the table's `Tile`, `DataList`, and
//!   `HeaderStorageBucket` storage objects and carries the table name; its count
//!   equals the number of tables in the document.
//!
//! Other in-stream types (text storages, drawables, styles, formats) are
//! intentionally omitted until their identity is confirmed the same way rather
//! than guessed from a single document.

/// Returns the role name for a known Numbers message type identifier.
///
/// The name describes the object's role as evidenced by its ZIP entry name or
/// its position in the reference graph (see the module docs); it is not Apple's
/// internal protobuf message name. Returns `None` for type identifiers that have
/// not been grounded in structural evidence yet.
pub fn message_type_name(message_type: u64) -> Option<&'static str> {
    let name = match message_type {
        1 => "Document",
        2 => "Sheet",
        210 => "ViewState",
        213 => "AnnotationAuthorStorage",
        401 => "DocumentStylesheet",
        4000 => "CalculationEngine",
        4008 => "FormulaRecord",
        4009 => "FormulaAuxiliaryRecord",
        6000 => "TableInfo",
        6001 => "TableModel",
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
        assert_eq!(message_type_name(4008), Some("FormulaRecord"));
        assert_eq!(message_type_name(4009), Some("FormulaAuxiliaryRecord"));
    }

    #[test]
    fn names_table_chain_types() {
        assert_eq!(message_type_name(2), Some("Sheet"));
        assert_eq!(message_type_name(6000), Some("TableInfo"));
        assert_eq!(message_type_name(6001), Some("TableModel"));
    }

    #[test]
    fn returns_none_for_unknown_types() {
        assert_eq!(message_type_name(0), None);
        assert_eq!(message_type_name(999_999), None);
    }
}
