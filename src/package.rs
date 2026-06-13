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
//! - `entry_bytes` supports stored (`0`) and deflated (`8`) ZIP members

use std::cell::OnceCell;
use std::fs;
use std::path::Path;

use crate::inspect::{InspectionReport, count_keywords};
use crate::plist::{PropertiesPlist, parse_properties_plist};
use crate::{DocumentKind, Error};

const EOCD_SIGNATURE: u32 = 0x0605_4B50;
const CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4B50;
const LOCAL_FILE_SIGNATURE: u32 = 0x0403_4B50;
const STORED_COMPRESSION_METHOD: u16 = 0;
const DEFLATE_COMPRESSION_METHOD: u16 = 8;

/// One file entry from a ZIP central directory.
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
    inflated_bytes: OnceCell<Result<Box<[u8]>, String>>,
}

/// A parsed iWork package (ZIP archive).
///
/// Reads the ZIP central directory on construction and caches it for
/// O(n) entry lookup. The raw bytes remain in memory so that [`Self::entry_bytes`]
/// can slice into them directly without copying for stored entries.
#[derive(Debug, Clone)]
pub struct Package {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}

/// Builds a minimal iWork-compatible ZIP archive from scratch.
///
/// Entries are always stored uncompressed (method 0). Call [`Self::add_entry`]
/// one or more times, then [`Self::finish`] to obtain the raw ZIP bytes.
#[derive(Debug, Clone, Default)]
pub struct PackageWriter {
    entries: Vec<PackageEntryWriter>,
}

#[derive(Debug, Clone)]
struct PackageEntryWriter {
    path: String,
    contents: Vec<u8>,
}

/// Classification of a package's ZIP layout against the variants this crate
/// currently handles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PackageSupport {
    /// The package contains `Index/*.iwa` entries directly — the modern layout
    /// this crate fully supports.
    SupportedDirectIndexEntries,
    /// The package contains a single `Index.zip` sub-archive — the legacy
    /// layout produced by very old versions of iWork.
    UnsupportedLegacyIndexZip,
    /// The package layout is unrecognised.
    #[default]
    UnsupportedUnknownLayout,
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
                inflated_bytes: OnceCell::new(),
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

    /// Classifies the package layout against the variants this crate
    /// currently recognizes.
    pub fn support(&self) -> PackageSupport {
        let has_direct_index_entries = self
            .entries
            .iter()
            .any(|entry| entry.path.starts_with("Index/"));
        if has_direct_index_entries {
            return PackageSupport::SupportedDirectIndexEntries;
        }

        let has_legacy_index_zip = self.entries.iter().any(|entry| entry.path == "Index.zip");
        if has_legacy_index_zip {
            return PackageSupport::UnsupportedLegacyIndexZip;
        }

        PackageSupport::UnsupportedUnknownLayout
    }

    /// Returns the raw bytes for a package entry.
    ///
    /// This reader currently supports stored (`compression_method == 0`) and
    /// deflated (`compression_method == 8`) ZIP members.
    pub fn entry_bytes(&self, path: &str) -> Result<&[u8], Error> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .ok_or_else(|| Error::MissingEntry(path.to_owned()))?;

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

        let data = self
            .bytes
            .get(data_start..data_end)
            .ok_or(Error::Truncated("file contents"))?;

        match entry.compression_method {
            STORED_COMPRESSION_METHOD => Ok(data),
            DEFLATE_COMPRESSION_METHOD => {
                let inflated = entry.inflated_bytes.get_or_init(|| {
                    inflate_raw(data, entry.uncompressed_size as usize)
                        .map(Vec::into_boxed_slice)
                        .map_err(|()| entry.path.clone())
                });
                inflated
                    .as_ref()
                    .map(Box::as_ref)
                    .map_err(|path| Error::InvalidCompressedEntry { path: path.clone() })
            }
            method => Err(Error::UnsupportedCompression {
                path: entry.path.clone(),
                method,
            }),
        }
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
            support: self.support(),
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

impl PackageWriter {
    /// Create an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an entry with the given path and raw contents.
    ///
    /// Entries are stored in the order they are added; the ZIP central
    /// directory records them in that same order.
    pub fn add_entry(
        &mut self,
        path: impl Into<String>,
        contents: impl Into<Vec<u8>>,
    ) -> &mut Self {
        self.entries.push(PackageEntryWriter {
            path: path.into(),
            contents: contents.into(),
        });
        self
    }

    /// Serialise all added entries to a valid ZIP archive.
    pub fn finish(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        let mut central_directory = Vec::new();
        let mut offsets = Vec::with_capacity(self.entries.len());

        for entry in &self.entries {
            let local_header_offset =
                u32::try_from(out.len()).map_err(|_| Error::InvalidCentralDirectory)?;
            offsets.push(local_header_offset);

            let path_bytes = entry.path.as_bytes();
            let crc32 = crc32(&entry.contents);
            let size =
                u32::try_from(entry.contents.len()).map_err(|_| Error::InvalidCentralDirectory)?;
            let path_len =
                u16::try_from(path_bytes.len()).map_err(|_| Error::InvalidCentralDirectory)?;

            out.extend_from_slice(&LOCAL_FILE_SIGNATURE.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&STORED_COMPRESSION_METHOD.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&crc32.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&path_len.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(path_bytes);
            out.extend_from_slice(&entry.contents);
        }

        let central_directory_offset =
            u32::try_from(out.len()).map_err(|_| Error::InvalidCentralDirectory)?;

        for (entry, offset) in self.entries.iter().zip(offsets) {
            let path_bytes = entry.path.as_bytes();
            let crc32 = crc32(&entry.contents);
            let size =
                u32::try_from(entry.contents.len()).map_err(|_| Error::InvalidCentralDirectory)?;
            let path_len =
                u16::try_from(path_bytes.len()).map_err(|_| Error::InvalidCentralDirectory)?;

            central_directory.extend_from_slice(&CENTRAL_DIRECTORY_SIGNATURE.to_le_bytes());
            central_directory.extend_from_slice(&20u16.to_le_bytes());
            central_directory.extend_from_slice(&20u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&STORED_COMPRESSION_METHOD.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&crc32.to_le_bytes());
            central_directory.extend_from_slice(&size.to_le_bytes());
            central_directory.extend_from_slice(&size.to_le_bytes());
            central_directory.extend_from_slice(&path_len.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u32.to_le_bytes());
            central_directory.extend_from_slice(&offset.to_le_bytes());
            central_directory.extend_from_slice(path_bytes);
        }

        let central_directory_size =
            u32::try_from(central_directory.len()).map_err(|_| Error::InvalidCentralDirectory)?;
        let entry_count =
            u16::try_from(self.entries.len()).map_err(|_| Error::InvalidCentralDirectory)?;

        out.extend_from_slice(&central_directory);
        out.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&entry_count.to_le_bytes());
        out.extend_from_slice(&entry_count.to_le_bytes());
        out.extend_from_slice(&central_directory_size.to_le_bytes());
        out.extend_from_slice(&central_directory_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());

        Ok(out)
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

fn inflate_raw(bytes: &[u8], expected_len: usize) -> Result<Vec<u8>, ()> {
    let mut output = vec![0u8; expected_len];
    let mut stream = ZStream {
        next_in: bytes.as_ptr(),
        avail_in: u32::try_from(bytes.len()).map_err(|_| ())?,
        next_out: output.as_mut_ptr(),
        avail_out: u32::try_from(output.len()).map_err(|_| ())?,
        ..ZStream::default()
    };

    let init_code = unsafe {
        inflateInit2_(
            &raw mut stream,
            -MAX_WBITS,
            zlibVersion(),
            i32::try_from(std::mem::size_of::<ZStream>()).map_err(|_| ())?,
        )
    };
    if init_code != Z_OK {
        return Err(());
    }

    let inflate_code = unsafe { inflate(&raw mut stream, Z_FINISH) };
    let end_code = unsafe { inflateEnd(&raw mut stream) };

    if inflate_code != Z_STREAM_END || end_code != Z_OK {
        return Err(());
    }

    let written = usize::try_from(stream.total_out).map_err(|_| ())?;
    if written != expected_len {
        return Err(());
    }
    output.truncate(written);
    Ok(output)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut value = 0xffff_ffffu32;
    for &byte in bytes {
        value ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (value & 1).wrapping_neg() & 0xedb8_8320;
            value = (value >> 1) ^ mask;
        }
    }
    !value
}

const Z_OK: i32 = 0;
const Z_STREAM_END: i32 = 1;
const Z_FINISH: i32 = 4;
const MAX_WBITS: i32 = 15;

#[repr(C)]
#[derive(Default)]
struct ZStream {
    next_in: *const u8,
    avail_in: u32,
    total_in: u64,
    next_out: *mut u8,
    avail_out: u32,
    total_out: u64,
    msg: *const i8,
    state: *mut std::ffi::c_void,
    zalloc: Option<unsafe extern "C" fn(*mut std::ffi::c_void, u32, u32) -> *mut std::ffi::c_void>,
    zfree: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void)>,
    opaque: *mut std::ffi::c_void,
    data_type: i32,
    adler: u64,
    reserved: u64,
}

#[link(name = "z")]
unsafe extern "C" {
    fn zlibVersion() -> *const i8;
    fn inflateInit2_(
        strm: *mut ZStream,
        window_bits: i32,
        version: *const i8,
        stream_size: i32,
    ) -> i32;
    fn inflate(strm: *mut ZStream, flush: i32) -> i32;
    fn inflateEnd(strm: *mut ZStream) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_entry_zip(path: &str, contents: &[u8]) -> Vec<u8> {
        PackageWriter::new()
            .add_entry(path, contents.to_vec())
            .finish()
            .unwrap()
    }

    // ── PackageWriter ────────────────────────────────────────────────────────

    #[test]
    fn writer_roundtrip_single_entry() {
        let data = b"hello iwork";
        let zip = single_entry_zip("Index/Document.iwa", data);
        let pkg = Package::from_bytes(zip).unwrap();
        assert_eq!(pkg.entries().len(), 1);
        assert_eq!(pkg.entries()[0].path, "Index/Document.iwa");
        assert_eq!(pkg.entry_bytes("Index/Document.iwa").unwrap(), data);
    }

    #[test]
    fn writer_roundtrip_multiple_entries() {
        let zip = PackageWriter::new()
            .add_entry("Index/Document.iwa", b"first".to_vec())
            .add_entry("Metadata/Properties.plist", b"second".to_vec())
            .finish()
            .unwrap();
        let pkg = Package::from_bytes(zip).unwrap();
        assert_eq!(pkg.entries().len(), 2);
        assert_eq!(pkg.entry_bytes("Index/Document.iwa").unwrap(), b"first");
        assert_eq!(pkg.entry_bytes("Metadata/Properties.plist").unwrap(), b"second");
    }

    #[test]
    fn writer_empty_contents_entry() {
        let zip = single_entry_zip("empty.iwa", b"");
        let pkg = Package::from_bytes(zip).unwrap();
        assert_eq!(pkg.entry_bytes("empty.iwa").unwrap(), b"");
    }

    #[test]
    fn writer_crc32_roundtrip_integrity() {
        // Verify that reading back a non-trivial payload returns exactly what was written.
        let data: Vec<u8> = (0u8..=255).collect();
        let zip = single_entry_zip("blob.iwa", &data);
        let pkg = Package::from_bytes(zip).unwrap();
        assert_eq!(pkg.entry_bytes("blob.iwa").unwrap(), data.as_slice());
    }

    // ── PackageSupport classification ────────────────────────────────────────

    #[test]
    fn support_direct_index_entries() {
        let zip = single_entry_zip("Index/Document.iwa", b"x");
        let pkg = Package::from_bytes(zip).unwrap();
        assert_eq!(pkg.support(), PackageSupport::SupportedDirectIndexEntries);
    }

    #[test]
    fn support_legacy_index_zip() {
        let zip = single_entry_zip("Index.zip", b"x");
        let pkg = Package::from_bytes(zip).unwrap();
        assert_eq!(pkg.support(), PackageSupport::UnsupportedLegacyIndexZip);
    }

    #[test]
    fn support_unknown_layout() {
        let zip = single_entry_zip("Metadata/Properties.plist", b"x");
        let pkg = Package::from_bytes(zip).unwrap();
        assert_eq!(pkg.support(), PackageSupport::UnsupportedUnknownLayout);
    }

    // ── entry_bytes error paths ───────────────────────────────────────────────

    #[test]
    fn entry_bytes_missing_entry_error() {
        let zip = single_entry_zip("Index/Document.iwa", b"x");
        let pkg = Package::from_bytes(zip).unwrap();
        let result = pkg.entry_bytes("Index/Missing.iwa");
        assert!(matches!(result, Err(Error::MissingEntry(p)) if p == "Index/Missing.iwa"));
    }

    #[test]
    fn from_bytes_not_a_zip_fails() {
        let result = Package::from_bytes(b"not a zip file".to_vec());
        assert!(matches!(result, Err(Error::NotAZipArchive)));
    }

    #[test]
    fn from_bytes_empty_fails() {
        let result = Package::from_bytes(vec![]);
        assert!(matches!(result, Err(Error::NotAZipArchive)));
    }

    // ── crc32 ────────────────────────────────────────────────────────────────

    #[test]
    fn crc32_empty_is_known_value() {
        // CRC32 of an empty byte sequence is 0x00000000.
        assert_eq!(crc32(b""), 0x0000_0000);
    }

    #[test]
    fn crc32_known_value() {
        // CRC32("123456789") = 0xCBF43926 — the standard check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
