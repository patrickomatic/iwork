//! Reader for Apple iWork packages (`.numbers`, `.pages`, `.key`).
//!
//! Open a document with the app-specific types:
//!
//! - [`numbers::Document`] — spreadsheet data: sheets, tables, cell values
//! - [`pages::Document`] — word-processor body: text fragments, media descriptions
//! - [`keynote::Document`] — presentation: slides, titles, text, media descriptions
//!
//! Use the generic [`Document`] when you only need the file-format version or
//! UUID and don't need app-specific content.

use std::path::Path;

mod error;
mod inspect;
mod plist;
mod stylesheet;

pub mod iwa;
pub mod keynote;
pub mod numbers;
pub mod package;
pub mod pages;
pub mod protobuf;

mod kind;

pub use error::Error;
pub use inspect::InspectionReport;
pub use kind::DocumentKind;
pub use package::PackageSupport;
pub use plist::PropertiesPlist;

/// A generic iWork package: format-version metadata and document-kind detection.
///
/// For app-specific content use [`numbers::Document`], [`pages::Document`], or
/// [`keynote::Document`] directly.
#[derive(Debug, Clone)]
pub struct Document {
    package: package::Package,
}

impl Document {
    /// Open a supported iWork package from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            package: package::Package::open(path)?,
        })
    }

    /// Open a supported iWork package from raw bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Ok(Self {
            package: package::Package::from_bytes(bytes)?,
        })
    }

    /// Produce a small inspection report: document kind, UUID, format version,
    /// and package layout classification.
    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}

#[cfg(test)]
mod tests;
