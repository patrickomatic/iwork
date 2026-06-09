//! Public entry points for reading Apple iWork packages.
//!
//! The crate currently supports generic package inspection plus app-specific
//! semantic readers:
//!
//! - [`Document`] for generic iWork package access
//! - [`Package`] for ZIP-level entry enumeration and raw entry bytes
//! - [`PropertiesPlist`] and [`InspectionReport`] for a small amount of
//!   reverse-engineered metadata extraction
//!
//! The file-format assumptions behind those APIs are documented in
//! `docs/file-format.md`, with parser-specific notes in `package.rs` and
//! `plist.rs`.

use std::path::Path;

mod error;
mod inspect;
mod iwa;
mod iwa_text;
mod kind;
mod package;
mod plist;
mod protobuf;
mod stylesheet;

pub mod keynote;
pub mod numbers;
pub mod pages;

pub use error::Error;
pub use inspect::{InspectionReport, count_keywords};
pub use iwa::{
    IwaArchive, IwaArchiveDescriptor, IwaChunk, IwaObject, IwaObjectReference, IwaPacket,
};
pub use kind::DocumentKind;
pub use package::{Entry, Package, PackageSupport, PackageWriter};
pub use plist::PropertiesPlist;
pub use protobuf::{ProtoField, ProtoMessage, ProtoValue};
pub use stylesheet::{OrderedF32, StyleAttributes, StyleRecord, StylesheetCatalog};

#[derive(Debug, Clone)]
pub struct Document {
    package: Package,
}

impl Document {
    /// Opens a supported iWork package from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            package: Package::open(path)?,
        })
    }

    /// Opens a supported iWork package from raw bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Ok(Self {
            package: Package::from_bytes(bytes)?,
        })
    }

    /// Returns the underlying ZIP-like package view.
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// Produces a small inspection report derived from known package members.
    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}
#[cfg(test)]
mod tests;
