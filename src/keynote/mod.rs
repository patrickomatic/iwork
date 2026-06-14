//! Keynote-specific entry points.
//!
//! The current reader exposes ordered UTF-8 string fields decoded from
//! slide-related archives.
//!
//! The parser is intentionally conservative and read-only. It does not classify
//! layout names, titles, media descriptions, presenter notes, or animations
//! until those Keynote object fields are decoded explicitly.

use std::path::Path;

use crate::package::Package;
use crate::{DocumentKind, Error, IWorkDocument, InspectionReport};

mod presentation;
mod write;

pub use presentation::{Presentation, Slide};

#[derive(Debug, Clone)]
pub struct Document {
    package: Package,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        if DocumentKind::from_path(path.as_ref()) != DocumentKind::Keynote {
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

    pub fn presentation(&self) -> Result<Presentation, Error> {
        Presentation::from_package(&self.package)
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
        self.presentation()?.to_keynote_bytes()
    }

    fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        Self::inspect(self, path)
    }
}
