use crate::iwa::IwaArchive;
use crate::{Error, Package, StylesheetCatalog};

const DOCUMENT_ENTRY: &str = "Index/Document.iwa";
const DOCUMENT_METADATA_ENTRY: &str = "Index/DocumentMetadata.iwa";
const METADATA_ENTRY: &str = "Index/Metadata.iwa";
const STYLESHEET_ENTRY: &str = "Index/DocumentStylesheet.iwa";

#[derive(Debug, Clone)]
pub struct DocumentModel {
    document: IwaArchive,
    document_metadata: IwaArchive,
    metadata: IwaArchive,
    stylesheet: IwaArchive,
    index_archives: Vec<IndexArchive>,
}

impl DocumentModel {
    pub(crate) fn from_package(package: &Package) -> Result<Self, Error> {
        let document = IwaArchive::decode(package.entry_bytes(DOCUMENT_ENTRY)?)?;
        let document_metadata = IwaArchive::decode(package.entry_bytes(DOCUMENT_METADATA_ENTRY)?)?;
        let metadata = IwaArchive::decode(package.entry_bytes(METADATA_ENTRY)?)?;
        let stylesheet = IwaArchive::decode(package.entry_bytes(STYLESHEET_ENTRY)?)?;

        let mut index_archives = package
            .entries()
            .iter()
            .filter(|entry| entry.path.starts_with("Index/") && entry.path.ends_with(".iwa"))
            .filter(|entry| {
                !matches!(
                    entry.path.as_str(),
                    DOCUMENT_ENTRY | DOCUMENT_METADATA_ENTRY | METADATA_ENTRY | STYLESHEET_ENTRY
                )
            })
            .map(|entry| {
                Ok(IndexArchive {
                    path: entry.path.clone(),
                    archive: IwaArchive::decode(package.entry_bytes(&entry.path)?)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        index_archives.sort_by(|left, right| left.path.cmp(&right.path));

        Ok(Self {
            document,
            document_metadata,
            metadata,
            stylesheet,
            index_archives,
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

    pub fn index_archives(&self) -> &[IndexArchive] {
        &self.index_archives
    }
}

#[derive(Debug, Clone)]
pub struct IndexArchive {
    path: String,
    archive: IwaArchive,
}

impl IndexArchive {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn archive(&self) -> &IwaArchive {
        &self.archive
    }
}
