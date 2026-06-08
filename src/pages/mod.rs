//! Pages-specific entry points.
//!
//! The current reader exposes two layers:
//!
//! - [`DocumentModel`] for decoded core `Index/*.iwa` archives
//! - [`SemanticDocument`] for best-effort extraction of user-facing text such as
//!   titles, headings, and ordered text fragments from `Index/Document.iwa`
//!
//! The semantic layer is intentionally conservative: it returns high-confidence
//! text and heading candidates from the current fixtures, but it is not yet a
//! full structural parse of Pages paragraph or text-run objects.

use std::path::Path;

use crate::{DocumentKind, Error, InspectionReport, Package};

mod document_model;
mod semantic;

pub use document_model::{DocumentModel, IndexArchive};
pub use semantic::SemanticDocument;

#[derive(Debug, Clone)]
pub struct Document {
    package: Package,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        if DocumentKind::from_path(path.as_ref()) != DocumentKind::Pages {
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

    pub fn document_model(&self) -> Result<DocumentModel, Error> {
        DocumentModel::from_package(&self.package)
    }

    pub fn semantic_document(&self) -> Result<SemanticDocument, Error> {
        SemanticDocument::from_package(&self.package)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.package.into_bytes()
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.package.write(path)
    }

    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}
