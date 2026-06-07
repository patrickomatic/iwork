use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

const EOCD_SIGNATURE: u32 = 0x0605_4B50;
const CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4B50;
const LOCAL_FILE_SIGNATURE: u32 = 0x0403_4B50;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NotAZipArchive,
    MissingEndOfCentralDirectory,
    InvalidCentralDirectory,
    InvalidLocalFileHeader,
    UnsupportedCompression { path: String, method: u16 },
    MissingEntry(String),
    InvalidUtf8(std::string::FromUtf8Error),
    InvalidPlist(&'static str),
    Truncated(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::NotAZipArchive => write!(f, "file does not look like a zip archive"),
            Self::MissingEndOfCentralDirectory => {
                write!(f, "could not find end-of-central-directory record")
            }
            Self::InvalidCentralDirectory => write!(f, "invalid central directory"),
            Self::InvalidLocalFileHeader => write!(f, "invalid local file header"),
            Self::UnsupportedCompression { path, method } => {
                write!(
                    f,
                    "entry {path} uses unsupported compression method {method}"
                )
            }
            Self::MissingEntry(path) => write!(f, "missing entry: {path}"),
            Self::InvalidUtf8(error) => write!(f, "invalid UTF-8: {error}"),
            Self::InvalidPlist(message) => write!(f, "invalid plist: {message}"),
            Self::Truncated(section) => write!(f, "truncated {section}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::InvalidUtf8(value)
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub compression_method: u16,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    local_header_offset: u32,
}

#[derive(Debug, Clone)]
pub struct Package {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertiesPlist {
    pub document_uuid: Option<String>,
    pub file_format_version: Option<String>,
    pub is_multi_page: Option<bool>,
    pub revision: Option<String>,
    pub stable_document_uuid: Option<String>,
    pub version_uuid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InspectionReport {
    pub path: String,
    pub properties: PropertiesPlist,
    pub entry_count: usize,
    pub iwa_count: usize,
    pub style_keyword_hits: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct Document {
    package: Package,
}

impl Document {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
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

    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        self.package.inspect(path)
    }
}

impl Package {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

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

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

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

    pub fn properties(&self) -> Result<PropertiesPlist, Error> {
        let bytes = self.entry_bytes("Metadata/Properties.plist")?;
        parse_properties_plist(bytes)
    }

    pub fn inspect(&self, path: impl Into<String>) -> Result<InspectionReport, Error> {
        let properties = self.properties()?;
        let stylesheet = self.entry_bytes("Index/DocumentStylesheet.iwa")?;
        let style_keyword_hits = count_keywords(
            stylesheet,
            &["bold", "italic", "underline", "strikethrough", "font"],
        );

        Ok(InspectionReport {
            path: path.into(),
            properties,
            entry_count: self.entries.len(),
            iwa_count: self
                .entries
                .iter()
                .filter(|entry| entry.path.ends_with(".iwa"))
                .count(),
            style_keyword_hits,
        })
    }
}

pub mod pages {
    use std::path::Path;

    use crate::{Error, Package};

    #[derive(Debug, Clone)]
    pub struct Document {
        package: Package,
    }

    impl Document {
        pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
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
    }
}

pub mod keynote {
    use std::path::Path;

    use crate::{Error, Package};

    #[derive(Debug, Clone)]
    pub struct Document {
        package: Package,
    }

    impl Document {
        pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
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
    }
}

pub fn count_keywords(bytes: &[u8], keywords: &[&str]) -> BTreeMap<String, usize> {
    let haystack = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    keywords
        .iter()
        .map(|keyword| {
            let needle = keyword.to_ascii_lowercase();
            let hits = haystack.match_indices(&needle).count();
            (needle, hits)
        })
        .collect()
}

fn parse_properties_plist(bytes: &[u8]) -> Result<PropertiesPlist, Error> {
    if bytes.starts_with(b"bplist00") {
        return parse_binary_properties_plist(bytes);
    }

    let xml = std::str::from_utf8(bytes).map_err(|_| Error::InvalidPlist("expected UTF-8"))?;
    parse_xml_properties_plist(xml)
}

fn parse_xml_properties_plist(xml: &str) -> Result<PropertiesPlist, Error> {
    let mut bools = BTreeMap::new();
    let mut strings = BTreeMap::new();
    let dict_start = xml
        .find("<dict>")
        .ok_or(Error::InvalidPlist("missing <dict>"))?;
    let dict_end = xml
        .rfind("</dict>")
        .ok_or(Error::InvalidPlist("missing </dict>"))?;
    let mut rest = &xml[dict_start + "<dict>".len()..dict_end];

    while let Some(key_start) = rest.find("<key>") {
        rest = &rest[key_start + "<key>".len()..];
        let key_end = rest
            .find("</key>")
            .ok_or(Error::InvalidPlist("unterminated <key>"))?;
        let key = &rest[..key_end];
        rest = &rest[key_end + "</key>".len()..];
        let value = rest.trim_start();

        if let Some(stripped) = value.strip_prefix("<string>") {
            let value_end = stripped
                .find("</string>")
                .ok_or(Error::InvalidPlist("unterminated <string>"))?;
            strings.insert(key.to_owned(), stripped[..value_end].to_owned());
            rest = &stripped[value_end + "</string>".len()..];
            continue;
        }

        if let Some(stripped) = value.strip_prefix("<true/>") {
            bools.insert(key.to_owned(), true);
            rest = stripped;
            continue;
        }

        if let Some(stripped) = value.strip_prefix("<false/>") {
            bools.insert(key.to_owned(), false);
            rest = stripped;
            continue;
        }

        return Err(Error::InvalidPlist("unsupported value type"));
    }

    Ok(PropertiesPlist {
        document_uuid: strings.remove("documentUUID"),
        file_format_version: strings.remove("fileFormatVersion"),
        is_multi_page: bools.remove("isMultiPage"),
        revision: strings.remove("revision"),
        stable_document_uuid: strings.remove("stableDocumentUUID"),
        version_uuid: strings.remove("versionUUID"),
    })
}

fn parse_binary_properties_plist(bytes: &[u8]) -> Result<PropertiesPlist, Error> {
    if bytes.len() < 40 {
        return Err(Error::InvalidPlist("binary plist is too short"));
    }

    let trailer = &bytes[bytes.len() - 32..];
    let offset_int_size = trailer[6] as usize;
    let object_ref_size = trailer[7] as usize;
    let num_objects = read_be_u64(trailer, 8)? as usize;
    let top_object = read_be_u64(trailer, 16)? as usize;
    let offset_table_offset = read_be_u64(trailer, 24)? as usize;

    if offset_int_size == 0 || object_ref_size == 0 {
        return Err(Error::InvalidPlist("invalid trailer sizes"));
    }

    let offset_table_size = num_objects
        .checked_mul(offset_int_size)
        .ok_or(Error::InvalidPlist("offset table overflow"))?;
    let offset_table_end = offset_table_offset
        .checked_add(offset_table_size)
        .ok_or(Error::InvalidPlist("offset table overflow"))?;
    if offset_table_end > bytes.len() - 32 {
        return Err(Error::InvalidPlist("offset table out of bounds"));
    }

    let mut offsets = Vec::with_capacity(num_objects);
    for index in 0..num_objects {
        let start = offset_table_offset + index * offset_int_size;
        offsets.push(read_be_usize(bytes, start, offset_int_size)?);
    }

    let object = parse_binary_plist_object(bytes, &offsets, object_ref_size, top_object)?;
    let dict = match object {
        BinaryPlistObject::Dict(dict) => dict,
        _ => return Err(Error::InvalidPlist("top object is not a dictionary")),
    };

    Ok(PropertiesPlist {
        document_uuid: dict_get_string(&dict, "documentUUID"),
        file_format_version: dict_get_string(&dict, "fileFormatVersion"),
        is_multi_page: dict_get_bool(&dict, "isMultiPage"),
        revision: dict_get_string(&dict, "revision"),
        stable_document_uuid: dict_get_string(&dict, "stableDocumentUUID"),
        version_uuid: dict_get_string(&dict, "versionUUID"),
    })
}

#[derive(Debug, Clone)]
enum BinaryPlistObject {
    String(String),
    Bool(bool),
    Dict(BTreeMap<String, BinaryPlistObject>),
}

fn dict_get_string(dict: &BTreeMap<String, BinaryPlistObject>, key: &str) -> Option<String> {
    match dict.get(key) {
        Some(BinaryPlistObject::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn dict_get_bool(dict: &BTreeMap<String, BinaryPlistObject>, key: &str) -> Option<bool> {
    match dict.get(key) {
        Some(BinaryPlistObject::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn parse_binary_plist_object(
    bytes: &[u8],
    offsets: &[usize],
    object_ref_size: usize,
    object_index: usize,
) -> Result<BinaryPlistObject, Error> {
    let offset = *offsets
        .get(object_index)
        .ok_or(Error::InvalidPlist("object index out of bounds"))?;
    let marker = *bytes
        .get(offset)
        .ok_or(Error::InvalidPlist("object offset out of bounds"))?;
    let object_type = marker >> 4;
    let object_info = (marker & 0x0F) as usize;

    match (object_type, object_info) {
        (0x0, 0x8) => Ok(BinaryPlistObject::Bool(false)),
        (0x0, 0x9) => Ok(BinaryPlistObject::Bool(true)),
        (0x5, _) => {
            let (len, data_start) = parse_plist_length(bytes, offset, object_info)?;
            let data_end = data_start
                .checked_add(len)
                .ok_or(Error::InvalidPlist("string length overflow"))?;
            let value = bytes
                .get(data_start..data_end)
                .ok_or(Error::InvalidPlist("ascii string out of bounds"))?;
            Ok(BinaryPlistObject::String(
                String::from_utf8(value.to_vec()).map_err(Error::InvalidUtf8)?,
            ))
        }
        (0x6, _) => {
            let (len, data_start) = parse_plist_length(bytes, offset, object_info)?;
            let byte_len = len
                .checked_mul(2)
                .ok_or(Error::InvalidPlist("utf16 string length overflow"))?;
            let data_end = data_start
                .checked_add(byte_len)
                .ok_or(Error::InvalidPlist("utf16 string length overflow"))?;
            let data = bytes
                .get(data_start..data_end)
                .ok_or(Error::InvalidPlist("utf16 string out of bounds"))?;
            let code_units = data
                .chunks_exact(2)
                .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                .collect::<Vec<_>>();
            let value = String::from_utf16(&code_units)
                .map_err(|_| Error::InvalidPlist("invalid utf16 string"))?;
            Ok(BinaryPlistObject::String(value))
        }
        (0xD, _) => {
            let (len, refs_start) = parse_plist_length(bytes, offset, object_info)?;
            let keys_start = refs_start;
            let values_start = keys_start
                .checked_add(
                    len.checked_mul(object_ref_size)
                        .ok_or(Error::InvalidPlist("dictionary reference overflow"))?,
                )
                .ok_or(Error::InvalidPlist("dictionary reference overflow"))?;

            let mut dict = BTreeMap::new();
            for index in 0..len {
                let key_ref =
                    read_be_usize(bytes, keys_start + index * object_ref_size, object_ref_size)?;
                let value_ref = read_be_usize(
                    bytes,
                    values_start + index * object_ref_size,
                    object_ref_size,
                )?;
                let key_object =
                    parse_binary_plist_object(bytes, offsets, object_ref_size, key_ref)?;
                let key = match key_object {
                    BinaryPlistObject::String(value) => value,
                    _ => return Err(Error::InvalidPlist("dictionary key is not a string")),
                };
                let value = parse_binary_plist_object(bytes, offsets, object_ref_size, value_ref)?;
                dict.insert(key, value);
            }

            Ok(BinaryPlistObject::Dict(dict))
        }
        _ => Err(Error::InvalidPlist("unsupported binary plist object")),
    }
}

fn parse_plist_length(
    bytes: &[u8],
    offset: usize,
    object_info: usize,
) -> Result<(usize, usize), Error> {
    if object_info < 0x0F {
        return Ok((object_info, offset + 1));
    }

    let int_marker = *bytes
        .get(offset + 1)
        .ok_or(Error::InvalidPlist("missing length integer"))?;
    if int_marker >> 4 != 0x1 {
        return Err(Error::InvalidPlist("length integer is not an int object"));
    }

    let int_power = (int_marker & 0x0F) as usize;
    let int_len = 1usize
        .checked_shl(int_power as u32)
        .ok_or(Error::InvalidPlist("length integer is too large"))?;
    let len_start = offset + 2;
    let len = read_be_usize(bytes, len_start, int_len)?;
    Ok((len, len_start + int_len))
}

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

fn read_be_usize(bytes: &[u8], offset: usize, width: usize) -> Result<usize, Error> {
    let slice = bytes
        .get(offset..offset + width)
        .ok_or(Error::InvalidPlist("integer out of bounds"))?;
    let mut value = 0usize;
    for byte in slice {
        value = value
            .checked_shl(8)
            .ok_or(Error::InvalidPlist("integer overflow"))?
            | (*byte as usize);
    }
    Ok(value)
}

fn read_be_u64(bytes: &[u8], offset: usize) -> Result<u64, Error> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or(Error::InvalidPlist("u64 out of bounds"))?;
    Ok(u64::from_be_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::{Document, Package, count_keywords, keynote, pages};

    #[test]
    fn counts_keyword_hits_case_insensitively() {
        let counts = count_keywords(b"Bold bold BOLD underline", &["bold", "underline"]);
        assert_eq!(counts["bold"], 3);
        assert_eq!(counts["underline"], 1);
    }

    #[test]
    fn parses_a_fixture_archive() {
        let package = Package::open("examples/personal_budget.numbers").unwrap();
        let properties = package.properties().unwrap();

        assert_eq!(properties.file_format_version.as_deref(), Some("14.4.1"));
        assert_eq!(properties.is_multi_page, Some(true));
        assert!(
            package
                .entries()
                .iter()
                .any(|entry| entry.path == "Index/DocumentStylesheet.iwa")
        );
    }

    #[test]
    fn app_specific_entry_points_share_the_core_package_reader() {
        let iwork_doc = Document::open("examples/personal_budget.numbers").unwrap();
        let pages_doc = pages::Document::open("examples/personal_budget.numbers").unwrap();
        let keynote_doc = keynote::Document::open("examples/personal_budget.numbers").unwrap();

        assert_eq!(
            iwork_doc.package().entries().len(),
            pages_doc.package().entries().len()
        );
        assert_eq!(
            pages_doc.package().entries().len(),
            keynote_doc.package().entries().len()
        );
    }
}
