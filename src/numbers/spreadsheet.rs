use std::collections::{BTreeMap, HashMap, HashSet};

use super::drawable::{SHEET_DRAWABLE_TYPE, SheetDrawable};
use super::formula::{FormulaAuxiliaryRecord, FormulaRecord};
use super::header_storage::{HeaderStorageBucket, TableHeaderStorage};
use super::sheet::{Sheet, table_info_to_model_ids};
use super::table::{
    CellFormat, CellValue, Table, decode_cell_format_datalist, decode_rich_text_datalist,
    decode_string_datalist,
};
use super::table_model::TableModel;
use super::types::message_type_name;
use crate::Error;
use crate::iwa::{IwaArchive, IwaObject};
use crate::package::Package;
use crate::protobuf::{ProtoMessage, read_varint};
use crate::stylesheet::StylesheetCatalog;

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";
const DOCUMENT_METADATA_ENTRY: &str = "Index/DocumentMetadata.iwa";
const METADATA_ENTRY: &str = "Index/Metadata.iwa";
const STYLESHEET_ENTRY: &str = "Index/DocumentStylesheet.iwa";
const CALCULATION_ENGINE_ENTRY: &str = "Index/CalculationEngine.iwa";
const TABLE_PREFIX: &str = "Index/Tables/";

/// All IWA archives from a Numbers package, decoded and ready for querying.
///
/// Construct via [`crate::numbers::Document::spreadsheet`].
#[derive(Debug, Clone)]
pub struct Spreadsheet {
    document: IwaArchive,
    document_metadata: IwaArchive,
    metadata: IwaArchive,
    stylesheet: IwaArchive,
    calculation_engine: IwaArchive,
    table_archives: Vec<TableArchive>,
}

impl Spreadsheet {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let document = IwaArchive::decode(package.entry_bytes(DOCUMENT_ENTRY)?)?;
        let document_metadata = IwaArchive::decode(package.entry_bytes(DOCUMENT_METADATA_ENTRY)?)?;
        let metadata = IwaArchive::decode(package.entry_bytes(METADATA_ENTRY)?)?;
        let stylesheet = IwaArchive::decode(package.entry_bytes(STYLESHEET_ENTRY)?)?;
        let calculation_engine =
            IwaArchive::decode(package.entry_bytes(CALCULATION_ENGINE_ENTRY)?)?;

        let mut table_archives = package
            .entries()
            .iter()
            .filter(|entry| entry.path.starts_with(TABLE_PREFIX))
            .map(|entry| {
                Ok(TableArchive {
                    path: entry.path.clone(),
                    archive: IwaArchive::decode(package.entry_bytes(&entry.path)?)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        table_archives.sort_by(|left, right| left.path.cmp(&right.path));

        Ok(Self {
            document,
            document_metadata,
            metadata,
            stylesheet,
            calculation_engine,
            table_archives,
        })
    }

    /// Decoded `Index/Document.iwa` archive (contains `Sheet` and `TableInfo` objects).
    pub fn document(&self) -> &IwaArchive {
        &self.document
    }

    /// Decoded `Index/DocumentMetadata.iwa` archive.
    pub fn document_metadata(&self) -> &IwaArchive {
        &self.document_metadata
    }

    /// Decoded `Index/Metadata.iwa` archive.
    pub fn metadata(&self) -> &IwaArchive {
        &self.metadata
    }

    /// Decoded `Index/DocumentStylesheet.iwa` archive.
    pub fn stylesheet(&self) -> &IwaArchive {
        &self.stylesheet
    }

    /// Decoded `Index/CalculationEngine.iwa` archive (contains `TableModel` objects).
    pub fn calculation_engine(&self) -> &IwaArchive {
        &self.calculation_engine
    }

    /// Decodes the document's tables from their `TableModel` objects.
    ///
    /// Each [`TableModel`] carries the table's name and geometry (row/column
    /// counts). This is the authoritative table list; unlike [`Spreadsheet::tables`],
    /// which decodes every `Tile` archive independently, it reflects the document's
    /// real tables. `TableModel` objects are stored in `Index/CalculationEngine.iwa`
    /// (with `Index/Document.iwa` scanned as a fallback for layouts that inline them).
    pub fn table_models(&self) -> Vec<TableModel> {
        let mut models = TableModel::collect(&self.calculation_engine);
        models.extend(TableModel::collect(&self.document));
        models.sort_by_key(TableModel::id);
        models.dedup_by_key(|model| model.id());
        models
    }

    /// Finds one table model by object id.
    pub fn table_model(&self, model_id: u64) -> Option<TableModel> {
        self.table_models()
            .into_iter()
            .find(|model| model.id() == model_id)
    }

    /// Resolves the table models that belong to a sheet, in sheet order.
    pub fn table_models_for_sheet(&self, sheet: &Sheet) -> Vec<TableModel> {
        sheet
            .table_model_ids()
            .iter()
            .filter_map(|model_id| self.table_model(*model_id))
            .collect()
    }

    /// Decodes formula records from `Index/CalculationEngine.iwa`.
    ///
    /// These are type-4008 objects whose field 2 matches formula ids preserved
    /// on some formula-result cells. The expression payload is not decoded yet,
    /// but this provides the first stable join from cells to formula records.
    pub fn formula_records(&self) -> Vec<FormulaRecord> {
        let mut records = FormulaRecord::collect(&self.calculation_engine);
        records.sort_by_key(FormulaRecord::formula_id);
        records
    }

    /// Finds a formula record by a local formula id stored on a formula cell.
    pub fn formula_record(&self, formula_id: u32) -> Option<FormulaRecord> {
        self.formula_records()
            .into_iter()
            .find(|record| record.formula_id() == formula_id)
    }

    /// Resolves a cached formula-result cell to its type-4008 formula record.
    pub fn formula_record_for_cell(&self, cell: &CellValue) -> Option<FormulaRecord> {
        cell.formula_id()
            .and_then(|formula_id| self.formula_record(formula_id))
    }

    /// Resolves all type-4008 formula records referenced by cells in a table.
    pub fn formula_records_for_table(&self, table: &Table) -> Vec<FormulaRecord> {
        let mut records = table
            .rows()
            .iter()
            .flat_map(|row| row.cells.iter())
            .filter_map(|cell| self.formula_record_for_cell(cell))
            .collect::<Vec<_>>();
        records.sort_by_key(FormulaRecord::formula_id);
        records.dedup_by_key(|record| record.formula_id());
        records
    }

    /// Resolves all formula records referenced by the decoded cells of a table model.
    pub fn formula_records_for_model(&self, model: &TableModel) -> Vec<FormulaRecord> {
        self.formula_records_for_table(&self.table(model))
    }

    /// Resolves all formula records referenced by the tables belonging to a sheet.
    pub fn formula_records_for_sheet(&self, sheet: &Sheet) -> Vec<FormulaRecord> {
        let models = self.table_models();
        let mut records = sheet
            .table_model_ids()
            .iter()
            .filter_map(|model_id| models.iter().find(|model| model.id() == *model_id))
            .flat_map(|model| self.formula_records_for_model(model))
            .collect::<Vec<_>>();
        records.sort_by_key(FormulaRecord::formula_id);
        records.dedup_by_key(|record| record.formula_id());
        records
    }

    /// Decodes type-4009 formula auxiliary records from `Index/CalculationEngine.iwa`.
    ///
    /// Type-4008 [`FormulaRecord`] objects reference these by object id. The
    /// record fields are exposed structurally while the exact formula-graph role
    /// is still under investigation.
    pub fn formula_auxiliary_records(&self) -> Vec<FormulaAuxiliaryRecord> {
        let mut records = FormulaAuxiliaryRecord::collect(&self.calculation_engine);
        records.sort_by_key(FormulaAuxiliaryRecord::object_id);
        records
    }

    /// Finds a type-4009 formula auxiliary record by object id.
    pub fn formula_auxiliary_record(&self, object_id: u64) -> Option<FormulaAuxiliaryRecord> {
        self.formula_auxiliary_records()
            .into_iter()
            .find(|record| record.object_id() == object_id)
    }

    /// Resolves the type-4009 auxiliary records referenced by one formula record.
    pub fn formula_auxiliary_records_for(
        &self,
        formula_record: &FormulaRecord,
    ) -> Vec<FormulaAuxiliaryRecord> {
        formula_record
            .auxiliary_record_ids()
            .iter()
            .filter_map(|object_id| self.formula_auxiliary_record(*object_id))
            .collect()
    }

    /// Resolves type-4009 auxiliary records referenced by formula cells in a table.
    pub fn formula_auxiliary_records_for_table(
        &self,
        table: &Table,
    ) -> Vec<FormulaAuxiliaryRecord> {
        let mut records = self
            .formula_records_for_table(table)
            .iter()
            .flat_map(|record| self.formula_auxiliary_records_for(record))
            .collect::<Vec<_>>();
        records.sort_by_key(FormulaAuxiliaryRecord::object_id);
        records.dedup_by_key(|record| record.object_id());
        records
    }

    /// Resolves type-4009 auxiliary records referenced by a table model's formula cells.
    pub fn formula_auxiliary_records_for_model(
        &self,
        model: &TableModel,
    ) -> Vec<FormulaAuxiliaryRecord> {
        self.formula_auxiliary_records_for_table(&self.table(model))
    }

    /// Resolves type-4009 auxiliary records referenced by formula cells in a sheet.
    pub fn formula_auxiliary_records_for_sheet(
        &self,
        sheet: &Sheet,
    ) -> Vec<FormulaAuxiliaryRecord> {
        let mut records = self
            .formula_records_for_sheet(sheet)
            .iter()
            .flat_map(|record| self.formula_auxiliary_records_for(record))
            .collect::<Vec<_>>();
        records.sort_by_key(FormulaAuxiliaryRecord::object_id);
        records.dedup_by_key(|record| record.object_id());
        records
    }

    /// Decodes the document's sheets and their table membership.
    ///
    /// Sheet objects live in `Index/Document.iwa`. Each sheet carries its
    /// display name and an ordered list of object references; filtering those
    /// references to `TableInfo` objects and resolving each `TableInfo` to its
    /// `TableModel` gives the sheet's table order.
    pub fn sheets(&self) -> Vec<Sheet> {
        let table_info_to_model_ids =
            table_info_to_model_ids(&[&self.document, &self.calculation_engine]);
        let mut sheets = Sheet::collect(&self.document, &table_info_to_model_ids);
        sheets.sort_by_key(Sheet::id);
        sheets
    }

    /// Resolves the owning sheet for one table-model object id.
    pub fn sheet_for_table_model(&self, model_id: u64) -> Option<Sheet> {
        self.sheets()
            .into_iter()
            .find(|sheet| sheet.table_model_ids().contains(&model_id))
    }

    /// Decodes sheet-level drawable objects referenced from the document's sheets.
    pub fn sheet_drawables(&self) -> Vec<SheetDrawable> {
        let mut drawables: Vec<SheetDrawable> = self
            .sheets()
            .into_iter()
            .flat_map(|sheet| sheet.non_table_object_reference_ids().collect::<Vec<_>>())
            .filter_map(|id| self.sheet_drawable(id))
            .collect();
        drawables.sort_by_key(SheetDrawable::object_id);
        drawables.dedup_by_key(|drawable| drawable.object_id());
        drawables
    }

    /// Decodes sheet-level drawables referenced by one sheet.
    pub fn drawables_for_sheet(&self, sheet: &Sheet) -> Vec<SheetDrawable> {
        let mut drawables: Vec<SheetDrawable> = sheet
            .non_table_object_reference_ids()
            .filter_map(|object_id| self.sheet_drawable(object_id))
            .collect();
        drawables.sort_by_key(SheetDrawable::object_id);
        drawables.dedup_by_key(|drawable| drawable.object_id());
        drawables
    }

    /// Resolves the owning sheet for one sheet-level drawable object id.
    pub fn sheet_for_drawable(&self, drawable_id: u64) -> Option<Sheet> {
        self.sheets().into_iter().find(|sheet| {
            sheet
                .non_table_object_reference_ids()
                .any(|object_id| object_id == drawable_id)
        })
    }

    /// Decodes one type-5021 sheet-level drawable by object id.
    pub fn sheet_drawable(&self, object_id: u64) -> Option<SheetDrawable> {
        let object = self.object_by_id(object_id)?;
        (object.message_type == Some(SHEET_DRAWABLE_TYPE))
            .then(|| SheetDrawable::from_object(&object))
            .flatten()
    }

    /// Raw 5020-5030 drawing/chart cluster objects referenced by a sheet drawable.
    pub fn sheet_drawable_objects(&self, drawable_id: u64) -> Vec<IwaObject> {
        self.object_references(drawable_id)
            .into_iter()
            .filter_map(|object_id| self.object_by_id(object_id))
            .filter(|object| object.message_type.is_some_and(is_drawable_cluster_type))
            .collect()
    }

    /// Resolved metadata for raw 5020-5030 objects referenced by a sheet drawable.
    pub fn sheet_drawable_object_info(&self, drawable_id: u64) -> Vec<ObjectInfo> {
        self.sheet_drawable_objects(drawable_id)
            .into_iter()
            .filter_map(|object| object.identifier)
            .filter_map(|object_id| self.object_info(object_id))
            .collect()
    }

    /// Counts raw 5020-5030 downstream object types referenced by a sheet drawable.
    pub fn sheet_drawable_cluster_type_counts(&self, drawable_id: u64) -> BTreeMap<u64, usize> {
        let mut counts = BTreeMap::new();
        for object in self.sheet_drawable_objects(drawable_id) {
            if let Some(message_type) = object.message_type {
                *counts.entry(message_type).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Heuristic style catalog decoded from `Index/DocumentStylesheet.iwa`.
    pub fn stylesheet_catalog(&self) -> StylesheetCatalog {
        StylesheetCatalog::from_archive(&self.stylesheet)
    }

    /// Decodes a `HeaderStorageBucket` archive by root object id.
    pub fn header_storage_bucket(&self, root_object_id: u64) -> Option<HeaderStorageBucket> {
        self.archive_by_root(root_object_id)
            .and_then(HeaderStorageBucket::from_archive)
    }

    /// Resolves the row- and column-indexed header storage buckets for a table.
    pub fn table_header_storage(&self, model: &TableModel) -> Option<TableHeaderStorage> {
        let row_bucket = self.header_storage_bucket(model.row_header_storage_bucket_id()?)?;
        let column_bucket = self.header_storage_bucket(model.column_header_storage_bucket_id()?)?;
        Some(TableHeaderStorage::new(row_bucket, column_bucket))
    }

    /// All decoded `Index/Tables/*.iwa` archives (tiles, data lists, etc.),
    /// sorted by path.
    pub fn table_archives(&self) -> &[TableArchive] {
        &self.table_archives
    }

    /// Resolves an object id to its iWork message type, if the object is present
    /// in one of the decoded Numbers archives.
    pub fn object_message_type(&self, object_id: u64) -> Option<u64> {
        self.object_by_id(object_id)
            .and_then(|object| object.message_type)
    }

    /// Finds a decoded object by id across the Numbers archive graph.
    pub fn object_by_id(&self, object_id: u64) -> Option<IwaObject> {
        self.core_archive_entries()
            .into_iter()
            .map(|(_, archive)| archive)
            .chain(self.table_archives.iter().map(|archive| &archive.archive))
            .flat_map(IwaArchive::objects)
            .find(|object| object.identifier == Some(object_id))
    }

    /// Decodes an object's raw payload as a protobuf message.
    pub fn object_message(&self, object_id: u64) -> Option<ProtoMessage> {
        self.object_by_id(object_id)
            .and_then(|object| ProtoMessage::decode(&object.payload).ok())
    }

    /// Resolves an object id into a compact graph summary.
    pub fn object_info(&self, object_id: u64) -> Option<ObjectInfo> {
        let message_type = self.object_message_type(object_id)?;
        let archive_path = self.object_archive_path(object_id)?.to_owned();
        Some(ObjectInfo {
            object_id,
            message_type,
            type_name: message_type_name(message_type),
            archive_path,
        })
    }

    /// Resolves an object id to the decoded `.iwa` archive path that contains it.
    pub fn object_archive_path(&self, object_id: u64) -> Option<&str> {
        self.core_archive_entries()
            .into_iter()
            .find_map(|(path, archive)| archive_has_object(archive, object_id).then_some(path))
            .or_else(|| {
                self.table_archives
                    .iter()
                    .find(|archive| archive_has_object(&archive.archive, object_id))
                    .map(TableArchive::path)
            })
    }

    /// Returns known package object ids referenced by the object's raw payload.
    ///
    /// This scans protobuf varints and retains only values that match another
    /// decoded object id in the same package.
    pub fn object_references(&self, object_id: u64) -> Vec<u64> {
        let Some(object) = self.object_by_id(object_id) else {
            return Vec::new();
        };
        let known_ids = self.known_object_ids();
        referenced_object_ids(&object.payload, object_id, &known_ids)
    }

    /// Resolves known package object references from an object's raw payload.
    pub fn object_reference_info(&self, object_id: u64) -> Vec<ObjectInfo> {
        self.object_references(object_id)
            .into_iter()
            .filter_map(|referenced_id| self.object_info(referenced_id))
            .collect()
    }

    /// Counts referenced object message types for any decoded object.
    pub fn object_reference_type_counts(&self, object_id: u64) -> BTreeMap<u64, usize> {
        let mut counts = BTreeMap::new();
        for info in self.object_reference_info(object_id) {
            *counts.entry(info.message_type()).or_insert(0) += 1;
        }
        counts
    }

    /// Resolves an object id to one of the currently grounded type names.
    pub fn object_message_type_name(&self, object_id: u64) -> Option<&'static str> {
        self.object_message_type(object_id)
            .and_then(message_type_name)
    }

    /// Decodes one table's cells, driven by its [`TableModel`].
    ///
    /// Unlike [`Spreadsheet::tables`], this gathers exactly the tiles the model
    /// references (merged in tile order) and resolves string cells through the
    /// model's own string `DataList`, so per-table string keys never collide
    /// across tables. Tiles or the string list missing from the package yield an
    /// empty contribution rather than an error.
    pub fn table(&self, model: &TableModel) -> Table {
        let strings = model
            .string_data_list_id()
            .and_then(|id| self.archive_by_root(id))
            .map(decode_string_datalist)
            .unwrap_or_default();
        let rich_texts = model
            .rich_text_data_list_id()
            .and_then(|id| self.archive_by_root(id))
            .map(decode_rich_text_datalist)
            .unwrap_or_default();
        let formats = model
            .cell_format_data_list_id()
            .and_then(|id| self.archive_by_root(id))
            .map(decode_cell_format_datalist)
            .unwrap_or_default();

        // Tiles span 256-row bands; each tile's rows carry a within-tile index, so
        // offset them by the tile's absolute starting row before merging.
        let mut rows = Vec::new();
        for (tile_id, row_offset) in model.tile_ids().iter().zip(model.tile_row_offsets()) {
            if let Some(tile) = self.archive_by_root(*tile_id) {
                for mut row in Table::from_tile(tile, &strings, &rich_texts, &formats).into_rows() {
                    row.index = u64::from(*row_offset).saturating_add(row.index);
                    rows.push(row);
                }
            }
        }
        Table::from_rows(rows)
    }

    /// Decodes every table in the document as `(model, cells)` pairs.
    ///
    /// This is the table-model-driven counterpart to [`Spreadsheet::tables`]: it
    /// returns one entry per real table, each carrying its name and geometry
    /// (via the [`TableModel`]) alongside its decoded rows.
    pub fn decoded_tables(&self) -> Vec<(TableModel, Table)> {
        self.table_models()
            .into_iter()
            .map(|model| {
                let table = self.table(&model);
                (model, table)
            })
            .collect()
    }

    /// Decodes the tables that belong to a sheet, preserving the sheet's table order.
    pub fn tables_for_sheet(&self, sheet: &Sheet) -> Vec<(TableModel, Table)> {
        self.table_models_for_sheet(sheet)
            .into_iter()
            .map(|model| {
                let table = self.table(&model);
                (model, table)
            })
            .collect()
    }

    /// Finds a decoded table archive (`Tile`, `DataList`, …) by its root object id.
    fn archive_by_root(&self, root_object_id: u64) -> Option<&IwaArchive> {
        self.table_archives
            .iter()
            .map(|table_archive| &table_archive.archive)
            .find(|archive| archive.descriptor().root_object_id == Some(root_object_id))
    }

    fn core_archive_entries(&self) -> [(&'static str, &IwaArchive); 5] {
        [
            (DOCUMENT_ENTRY, &self.document),
            (DOCUMENT_METADATA_ENTRY, &self.document_metadata),
            (METADATA_ENTRY, &self.metadata),
            (STYLESHEET_ENTRY, &self.stylesheet),
            (CALCULATION_ENGINE_ENTRY, &self.calculation_engine),
        ]
    }

    fn known_object_ids(&self) -> HashSet<u64> {
        self.core_archive_entries()
            .into_iter()
            .map(|(_, archive)| archive)
            .chain(self.table_archives.iter().map(|archive| &archive.archive))
            .flat_map(IwaArchive::objects)
            .filter_map(|object| object.identifier)
            .collect()
    }

    /// Decodes all table tiles in path order.
    ///
    /// String cells are resolved through any `DataList` archives found under
    /// `Index/Tables/`; numeric and date values are decoded inline from each
    /// tile row's cell-storage buffer.
    pub fn tables(&self) -> Vec<Table> {
        let strings: HashMap<u32, String> = self
            .table_archives
            .iter()
            .filter(|a| a.path.contains("DataList"))
            .flat_map(|a| decode_string_datalist(&a.archive))
            .collect();

        let empty_rich: HashMap<u32, String> = HashMap::new();
        let empty_fmt: HashMap<u32, CellFormat> = HashMap::new();
        self.table_archives
            .iter()
            .filter(|a| a.path.contains("Tile"))
            .map(|a| Table::from_tile(&a.archive, &strings, &empty_rich, &empty_fmt))
            .collect()
    }
}

fn archive_has_object(archive: &IwaArchive, object_id: u64) -> bool {
    archive
        .objects()
        .iter()
        .any(|object| object.identifier == Some(object_id))
}

fn referenced_object_ids(payload: &[u8], self_id: u64, known_ids: &HashSet<u64>) -> Vec<u64> {
    let mut referenced = Vec::new();
    for start in 0..payload.len() {
        let mut cursor = start;
        let Ok(value) = read_varint(payload, &mut cursor) else {
            continue;
        };
        if value != self_id && known_ids.contains(&value) && !referenced.contains(&value) {
            referenced.push(value);
        }
    }
    referenced
}

fn is_drawable_cluster_type(message_type: u64) -> bool {
    (5020..=5030).contains(&message_type)
}

#[derive(Debug, Clone)]
pub struct TableArchive {
    path: String,
    archive: IwaArchive,
}

/// Resolved metadata for one object in a Numbers package graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    object_id: u64,
    message_type: u64,
    type_name: Option<&'static str>,
    archive_path: String,
}

impl ObjectInfo {
    /// Object identifier within the package.
    pub fn object_id(&self) -> u64 {
        self.object_id
    }

    /// Raw iWork message type identifier.
    pub fn message_type(&self) -> u64 {
        self.message_type
    }

    /// Grounded role name for known message types.
    pub fn type_name(&self) -> Option<&'static str> {
        self.type_name
    }

    /// Package-relative `.iwa` path containing the object.
    pub fn archive_path(&self) -> &str {
        &self.archive_path
    }
}

impl TableArchive {
    /// Package-relative path such as `Index/Tables/Tile-....iwa`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Decoded IWA archive for the table-related entry.
    pub fn archive(&self) -> &IwaArchive {
        &self.archive
    }
}

#[cfg(test)]
mod tests {
    use crate::numbers;

    const PERSONAL_BUDGET: &str = "examples/numbers/personal_budget.numbers";
    const PIVOT_TABLE: &str = "examples/numbers/pivot_table.numbers";

    #[test]
    fn spreadsheet_exposes_named_archives() {
        let spreadsheet = numbers::Document::open(PERSONAL_BUDGET)
            .unwrap()
            .spreadsheet()
            .unwrap();
        // Each archive must contain at least one object.
        assert!(!spreadsheet.document().objects().is_empty());
        assert!(!spreadsheet.calculation_engine().objects().is_empty());
        assert!(!spreadsheet.stylesheet().objects().is_empty());
    }

    #[test]
    fn spreadsheet_has_table_archives() {
        let spreadsheet = numbers::Document::open(PERSONAL_BUDGET)
            .unwrap()
            .spreadsheet()
            .unwrap();
        assert!(!spreadsheet.table_archives().is_empty());
        assert!(
            spreadsheet
                .table_archives()
                .iter()
                .all(|a| a.path().starts_with("Index/Tables/")),
            "all table archives should live under Index/Tables/"
        );
    }

    #[test]
    fn spreadsheet_table_models_match_decoded_tables() {
        let spreadsheet = numbers::Document::open(PERSONAL_BUDGET)
            .unwrap()
            .spreadsheet()
            .unwrap();
        let models = spreadsheet.table_models();
        let decoded = spreadsheet.decoded_tables();
        assert_eq!(models.len(), decoded.len());
        for (model, (dm, _table)) in models.iter().zip(decoded.iter()) {
            assert_eq!(model.id(), dm.id());
        }
    }

    #[test]
    fn spreadsheet_sheets_have_names() {
        let spreadsheet = numbers::Document::open(PERSONAL_BUDGET)
            .unwrap()
            .spreadsheet()
            .unwrap();
        let sheets = spreadsheet.sheets();
        assert!(!sheets.is_empty());
        assert!(
            sheets.iter().all(|s| s.name().is_some()),
            "all sheets in personal_budget should have a name"
        );
    }

    #[test]
    fn spreadsheet_pivot_table_has_table_models() {
        let spreadsheet = numbers::Document::open(PIVOT_TABLE)
            .unwrap()
            .spreadsheet()
            .unwrap();
        assert!(!spreadsheet.table_models().is_empty());
    }

    #[test]
    fn stylesheet_catalog_has_font_names() {
        let spreadsheet = numbers::Document::open(PERSONAL_BUDGET)
            .unwrap()
            .spreadsheet()
            .unwrap();
        let catalog = spreadsheet.stylesheet_catalog();
        assert!(
            !catalog.font_names.is_empty(),
            "stylesheet should reference at least one font"
        );
    }
}
