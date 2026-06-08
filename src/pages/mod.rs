//! Pages-specific entry points.
//!
//! The current reader exposes a best-effort document view for user-facing text
//! such as titles, headings, and ordered text fragments from
//! `Index/Document.iwa`.
//!
//! The parser is intentionally conservative: it returns high-confidence
//! text and heading candidates from the current fixtures, but it is not yet a
//! full structural parse of Pages paragraph or text-run objects.

use std::path::Path;

use crate::{DocumentKind, Error, InspectionReport, Package};

mod semantic;

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

    pub fn document(&self) -> Result<SemanticDocument, Error> {
        SemanticDocument::from_package(&self.package)
    }

    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}
