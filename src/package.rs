//! ZIP-level package reader for iWork documents.
//!
//! The crate currently treats `.numbers`, `.pages`, and `.key` files as ZIP
//! archives and performs just enough parsing to:
//!
//! - enumerate central-directory entries
//! - look up raw bytes for selected package members
//! - read `Metadata/Properties.plist`
//! - inspect `Index/DocumentStylesheet.iwa`
//!
//! Important constraints:
//!
//! - only standard EOCD / central-directory / local-file-header records are
//!   supported
//! - entry names are expected to be UTF-8
//! - `entry_bytes` only supports stored entries with compression method `0`
//! - write support is byte-preserving rather than reconstructing the archive

use std::fs;
use std::path::Path;

use crate::inspect::{InspectionReport, count_keywords};
use crate::plist::{PropertiesPlist, parse_properties_plist};
use crate::{DocumentKind, Error};

const EOCD_SIGNATURE: u32 = 0x0605_4B50;
const CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4B50;
const LOCAL_FILE_SIGNATURE: u32 = 0x0403_4B50;

#[derive(Debug, Clone)]
pub struct Entry {
    /// Package-relative path exactly as stored in the ZIP central directory.
    pub path: String,
    /// ZIP compression method from the central directory record.
    pub compression_method: u16,
    /// Compressed size recorded for the entry.
    pub compressed_size: u32,
    /// Uncompressed size recorded for the entry.
    pub uncompressed_size: u32,
    local_header_offset: u32,
}

#[derive(Debug, Clone)]
pub struct Package {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}

impl Package {
    /// Reads an iWork package from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    /// Parses an iWork package from raw bytes.
    ///
    /// This validates the outer ZIP structure and records the central
    /// directory metadata needed for later entry access.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.len() < 4 || read_u32(&bytes, 0)? != LOCAL_FILE_SIGNATURE {
            return Err(Error::NotAZipArchive);
        }

        let eocd_offset = find_eocd(&bytes).ok_or(Error::MissingEndOfCentralDirectory)?;
        let entry_count = read_u16(&bytes, eocd_offset + 10)? as usize;
        let central_directory_size = read_u32(&bytes, eocd_offset + 12)? as usize;
        let central_directory_offset = read_u32(&bytes, eocd_offset + 16)? as usize;
        let central_directory_end = central_directory_offset
            .checked_add(central_directory_size)
            .ok_or(Error::InvalidCentralDirectory)?;

        if central_directory_end > bytes.len() {
            return Err(Error::InvalidCentralDirectory);
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut cursor = central_directory_offset;
        for _ in 0..entry_count {
            if read_u32(&bytes, cursor)? != CENTRAL_DIRECTORY_SIGNATURE {
                return Err(Error::InvalidCentralDirectory);
            }

            let compression_method = read_u16(&bytes, cursor + 10)?;
            let compressed_size = read_u32(&bytes, cursor + 20)?;
            let uncompressed_size = read_u32(&bytes, cursor + 24)?;
            let file_name_length = read_u16(&bytes, cursor + 28)? as usize;
            let extra_length = read_u16(&bytes, cursor + 30)? as usize;
            let comment_length = read_u16(&bytes, cursor + 32)? as usize;
            let local_header_offset = read_u32(&bytes, cursor + 42)?;

            let name_start = cursor + 46;
            let name_end = name_start
                .checked_add(file_name_length)
                .ok_or(Error::InvalidCentralDirectory)?;
            if name_end > bytes.len() {
                return Err(Error::InvalidCentralDirectory);
            }

            let path = String::from_utf8(bytes[name_start..name_end].to_vec())?;
            entries.push(Entry {
                path,
                compression_method,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            });

            cursor = name_end
                .checked_add(extra_length)
                .and_then(|value| value.checked_add(comment_length))
                .ok_or(Error::InvalidCentralDirectory)?;
        }

        Ok(Self { bytes, entries })
    }

    /// Returns the central-directory entries discovered in the package.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Consumes the package and returns the original archive bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Writes the original archive bytes back out unchanged.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        fs::write(path, &self.bytes)?;
        Ok(())
    }

    /// Returns the raw bytes for a package entry.
    ///
    /// This reader currently supports only stored ZIP members
    /// (`compression_method == 0`).
    pub fn entry_bytes(&self, path: &str) -> Result<&[u8], Error> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .ok_or_else(|| Error::MissingEntry(path.to_owned()))?;

        if entry.compression_method != 0 {
            return Err(Error::UnsupportedCompression {
                path: entry.path.clone(),
                method: entry.compression_method,
            });
        }

        let offset = entry.local_header_offset as usize;
        if read_u32(&self.bytes, offset)? != LOCAL_FILE_SIGNATURE {
            return Err(Error::InvalidLocalFileHeader);
        }

        let file_name_length = read_u16(&self.bytes, offset + 26)? as usize;
        let extra_length = read_u16(&self.bytes, offset + 28)? as usize;
        let data_start = offset
            .checked_add(30)
            .and_then(|value| value.checked_add(file_name_length))
            .and_then(|value| value.checked_add(extra_length))
            .ok_or(Error::InvalidLocalFileHeader)?;
        let data_end = data_start
            .checked_add(entry.compressed_size as usize)
            .ok_or(Error::InvalidLocalFileHeader)?;

        self.bytes
            .get(data_start..data_end)
            .ok_or(Error::Truncated("file contents"))
    }

    /// Reads and parses `Metadata/Properties.plist`.
    pub fn properties(&self) -> Result<PropertiesPlist, Error> {
        let bytes = self.entry_bytes("Metadata/Properties.plist")?;
        parse_properties_plist(bytes)
    }

    /// Produces a small report based on package members we currently
    /// understand.
    ///
    /// The report uses filename extension for app classification and treats
    /// `Index/DocumentStylesheet.iwa` as opaque bytes for keyword scanning.
    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        let properties = self.properties()?;
        let stylesheet = self.entry_bytes("Index/DocumentStylesheet.iwa")?;
        let path = path.into();
        let style_keyword_hits = count_keywords(
            stylesheet,
            &["bold", "italic", "underline", "strikethrough", "font"],
        );

        Ok(InspectionReport {
            kind: DocumentKind::from_path(&path),
            path,
            properties,
            entry_count: self.entries.len(),
            iwa_count: self
                .entries
                .iter()
                .filter(|entry| {
                    Path::new(&entry.path)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("iwa"))
                })
                .count(),
            style_keyword_hits,
        })
    }
}

/// Finds the ZIP end-of-central-directory record by scanning backward from the
/// end of the archive, allowing for an optional trailing comment.
fn find_eocd(bytes: &[u8]) -> Option<usize> {
    let start = bytes.len().saturating_sub(22 + 65_535);
    (start..=bytes.len().saturating_sub(4))
        .rev()
        .find(|&offset| read_u32(bytes, offset).ok() == Some(EOCD_SIGNATURE))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or(Error::Truncated("u16"))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or(Error::Truncated("u32"))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}
