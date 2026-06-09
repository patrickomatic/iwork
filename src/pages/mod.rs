//! Pages-specific entry points.
//!
//! The current reader exposes ordered UTF-8 string fields decoded from
//! `Index/Document.iwa`.
//!
//! The parser is intentionally conservative: it does not classify titles,
//! headings, paragraphs, or text runs until those Pages object fields are
//! decoded explicitly.

use std::path::Path;

use crate::{DocumentKind, Error, InspectionReport, Package};

mod body;

pub use body::Body;

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

    pub fn document(&self) -> Result<Body, Error> {
        Body::from_package(&self.package)
    }

    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}
