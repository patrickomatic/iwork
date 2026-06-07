use crate::iwa::IwaArchive;
use crate::{Error, Package};

use super::StylesheetCatalog;

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
}

#[derive(Debug, Clone)]
pub struct TableArchive {
    path: String,
    archive: IwaArchive,
}

impl TableArchive {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn archive(&self) -> &IwaArchive {
        &self.archive
    }
}
