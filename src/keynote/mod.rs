//! Keynote-specific entry points.
//!
//! The current reader exposes two layers:
//!
//! - [`Presentation`] for decoded core `Index/*.iwa` archives
//! - [`SemanticPresentation`] for best-effort extraction of slide text,
//!   placeholder titles, layout names, and media descriptions from slide archives
//!
//! The semantic layer is intentionally conservative and read-only. It is useful
//! for recovering presentation content from the current fixtures, but it is not
//! yet a full structural parse of Keynote slides, presenter notes, or animation
//! timelines.

use std::path::Path;

use crate::{DocumentKind, Error, InspectionReport, Package};

mod presentation;
mod semantic;

pub use presentation::{IndexArchive, Presentation};
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

    pub fn presentation(&self) -> Result<Presentation, Error> {
        Presentation::from_package(&self.package)
    }

    pub fn semantic_presentation(&self) -> Result<SemanticPresentation, Error> {
        SemanticPresentation::from_package(&self.package)
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
