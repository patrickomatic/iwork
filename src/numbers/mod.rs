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
//!   text, numbers, dates, and empty cells
//!
//! See `docs/file-format.md` for the reverse-engineered field layout used by
//! the row decoder.

use std::path::Path;

use crate::{DocumentKind, Error, InspectionReport, Package};

mod spreadsheet;
mod table;
mod table_model;
mod types;
mod write;

pub use spreadsheet::{Spreadsheet, TableArchive};
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
