//! Numbers-specific entry points and table-decoding types.
//!
//! The current reader exposes a [`Spreadsheet`] view backed by a handful of
//! core `.iwa` archives plus a higher-level table decoder:
//!
//! - `Spreadsheet::table_archives()` returns the raw `DataList` and `Tile`
//!   archives under `Index/Tables/`
//! - `Spreadsheet::tables()` resolves string `DataList` payloads and decodes
//!   table rows from `Tile` archives
//! - [`CellValue`] surfaces the currently supported scalar cell types:
//!   text, rich text, numbers, dates, booleans, durations, formula errors,
//!   cached formula results, currency, percentages, and empty cells
//!
//! See `docs/file-format.md` for the reverse-engineered field layout used by
//! the row decoder.

use std::path::Path;

use crate::package::Package;
use crate::{DocumentKind, Error, IWorkDocument, InspectionReport};

mod drawable;
mod formula;
mod header_storage;
mod sheet;
mod spreadsheet;
mod table;
mod table_model;
mod types;
mod write;

pub use drawable::SheetDrawable;
pub use formula::{
    FormulaAuxiliaryEntry, FormulaAuxiliaryEntryPayload, FormulaAuxiliaryRecord, FormulaBounds,
    FormulaBoundsPair, FormulaExpression, FormulaRecord, FormulaRecordKey,
};
pub use header_storage::{HeaderStorageBucket, HeaderStorageEntry, TableHeaderStorage};
pub use sheet::Sheet;
pub use spreadsheet::{ObjectInfo, Spreadsheet, TableArchive};
pub use table::{CellValue, Table, TableRow};
pub use table_model::TableModel;
pub use types::message_type_name;
pub use write::{EncodedTableArchive, Workbook, WritableTable};

#[derive(Debug, Clone)]
pub struct Document {
    package: Package,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        if DocumentKind::from_path(path.as_ref()) != DocumentKind::Numbers {
            return Err(Error::UnsupportedDocumentType(
                path.as_ref().display().to_string(),
            ));
        }
        Ok(Self {
            package: Package::open(path)?,
        })
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Ok(Self {
            package: Package::from_bytes(bytes)?,
        })
    }

    pub fn package(&self) -> &Package {
        &self.package
    }

    /// Decodes the Numbers spreadsheet archives needed for table access.
    pub fn spreadsheet(&self) -> Result<Spreadsheet, Error> {
        Spreadsheet::from_package(&self.package)
    }

    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}

impl IWorkDocument for Document {
    fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::open(path)
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Self::from_bytes(bytes)
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        // Build a Workbook from the decoded spreadsheet and re-encode.
        let spreadsheet = self.spreadsheet()?;
        let mut workbook = write::Workbook::new();
        for (model, table) in spreadsheet.decoded_tables() {
            let name = model.name().unwrap_or("Table");
            let mut wt = write::WritableTable::new(name);
            for row in table.rows() {
                wt.push_row(row.cells.clone());
            }
            workbook.add_table(wt);
        }
        workbook.to_numbers_bytes()
    }

    fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        Self::inspect(self, path)
    }
}
