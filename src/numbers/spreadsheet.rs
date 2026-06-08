use std::collections::HashMap;

use super::table::{Table, decode_string_datalist};
use crate::iwa::IwaArchive;
use crate::{Error, Package, StylesheetCatalog};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";
const DOCUMENT_METADATA_ENTRY: &str = "Index/DocumentMetadata.iwa";
const METADATA_ENTRY: &str = "Index/Metadata.iwa";
const STYLESHEET_ENTRY: &str = "Index/DocumentStylesheet.iwa";
const TABLE_PREFIX: &str = "Index/Tables/";

#[derive(Debug, Clone)]
pub struct Spreadsheet {
    document: IwaArchive,
    document_metadata: IwaArchive,
    metadata: IwaArchive,
    stylesheet: IwaArchive,
    table_archives: Vec<TableArchive>,
}

impl Spreadsheet {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let document = IwaArchive::decode(package.entry_bytes(DOCUMENT_ENTRY)?)?;
        let document_metadata = IwaArchive::decode(package.entry_bytes(DOCUMENT_METADATA_ENTRY)?)?;
        let metadata = IwaArchive::decode(package.entry_bytes(METADATA_ENTRY)?)?;
        let stylesheet = IwaArchive::decode(package.entry_bytes(STYLESHEET_ENTRY)?)?;

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
            table_archives,
        })
    }

    pub fn document(&self) -> &IwaArchive {
        &self.document
    }

    pub fn document_metadata(&self) -> &IwaArchive {
        &self.document_metadata
    }

    pub fn metadata(&self) -> &IwaArchive {
        &self.metadata
    }

    pub fn stylesheet(&self) -> &IwaArchive {
        &self.stylesheet
    }

    pub fn stylesheet_catalog(&self) -> StylesheetCatalog {
        StylesheetCatalog::from_archive(&self.stylesheet)
    }

    pub fn table_archives(&self) -> &[TableArchive] {
        &self.table_archives
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

        self.table_archives
            .iter()
            .filter(|a| a.path.contains("Tile"))
            .map(|a| Table::from_tile(&a.archive, &strings))
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
