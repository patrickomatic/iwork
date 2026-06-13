use std::collections::HashMap;

use super::formula::FormulaRecord;
use super::header_storage::HeaderStorageBucket;
use super::sheet::{table_info_to_model_ids, Sheet};
use super::table::{
    decode_cell_format_datalist, decode_rich_text_datalist, decode_string_datalist, CellFormat,
    Table,
};
use super::table_model::TableModel;
use crate::iwa::IwaArchive;
use crate::package::Package;
use crate::stylesheet::StylesheetCatalog;
use crate::Error;

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

    /// Heuristic style catalog decoded from `Index/DocumentStylesheet.iwa`.
    pub fn stylesheet_catalog(&self) -> StylesheetCatalog {
        StylesheetCatalog::from_archive(&self.stylesheet)
    }

    /// Decodes a `HeaderStorageBucket` archive by root object id.
    pub fn header_storage_bucket(&self, root_object_id: u64) -> Option<HeaderStorageBucket> {
        self.archive_by_root(root_object_id)
            .and_then(HeaderStorageBucket::from_archive)
    }

    /// All decoded `Index/Tables/*.iwa` archives (tiles, data lists, etc.),
    /// sorted by path.
    pub fn table_archives(&self) -> &[TableArchive] {
        &self.table_archives
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

    /// Finds a decoded table archive (`Tile`, `DataList`, …) by its root object id.
    fn archive_by_root(&self, root_object_id: u64) -> Option<&IwaArchive> {
        self.table_archives
            .iter()
            .map(|table_archive| &table_archive.archive)
            .find(|archive| archive.descriptor().root_object_id == Some(root_object_id))
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

#[derive(Debug, Clone)]
pub struct TableArchive {
    path: String,
    archive: IwaArchive,
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
            spreadsheet.table_archives().iter().all(|a| a.path().starts_with("Index/Tables/")),
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
        assert!(!catalog.font_names.is_empty(), "stylesheet should reference at least one font");
    }
}
