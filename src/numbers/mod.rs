use std::path::Path;

use crate::{DocumentKind, Error, InspectionReport, Package};

mod spreadsheet;
mod styles;

pub use spreadsheet::{Spreadsheet, TableArchive};
pub use styles::StylesheetCatalog;

#[derive(Debug, Clone)]
pub struct Document {
    package: Package,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        if DocumentKind::from_path(path.as_ref()) != DocumentKind::Numbers {
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

    pub fn spreadsheet(&self) -> Result<Spreadsheet, Error> {
        Spreadsheet::from_package(&self.package)
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
