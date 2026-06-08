//! Keynote-specific entry points.
//!
//! The current reader exposes a best-effort presentation view for slide text,
//! placeholder titles, layout names, and media descriptions recovered from
//! slide archives.
//!
//! The parser is intentionally conservative and read-only. It is useful
//! for recovering presentation content from the current fixtures, but it is not
//! yet a full structural parse of Keynote slides, presenter notes, or animation
//! timelines.

use std::path::Path;

use crate::{DocumentKind, Error, InspectionReport, Package};

mod semantic;

pub use semantic::{SemanticPresentation, SemanticSlide};

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

    pub fn presentation(&self) -> Result<SemanticPresentation, Error> {
        SemanticPresentation::from_package(&self.package)
    }

    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}
