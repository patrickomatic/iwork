use std::fmt;

/// All errors that can arise when opening or reading an iWork package.
#[derive(Debug)]
pub enum Error {
    /// An OS-level I/O failure (file not found, permission denied, etc.).
    Io(std::io::Error),
    /// The file does not begin with a ZIP local-file-header signature.
    NotAZipArchive,
    /// The file extension is not one of `.numbers`, `.pages`, or `.key`.
    UnsupportedDocumentType(String),
    /// An IWA chunk carries an unrecognised kind byte (only `0` / Snappy is supported).
    UnsupportedIwaChunkType(u8),
    /// The end-of-central-directory (EOCD) record could not be located in the ZIP.
    MissingEndOfCentralDirectory,
    /// The ZIP central directory is malformed or its offsets are out of range.
    InvalidCentralDirectory,
    /// A ZIP local file header at the recorded offset has a bad signature or
    /// its field offsets are out of range.
    InvalidLocalFileHeader,
    /// An entry uses a ZIP compression method other than stored (0) or deflate (8).
    UnsupportedCompression { path: String, method: u16 },
    /// A deflate-compressed entry could not be decompressed.
    InvalidCompressedEntry { path: String },
    /// A requested package entry path was not found in the ZIP central directory.
    MissingEntry(String),
    /// A byte sequence that was expected to be valid UTF-8 was not.
    InvalidUtf8(std::string::FromUtf8Error),
    /// A `Metadata/Properties.plist` value is missing or has the wrong type.
    InvalidPlist(&'static str),
    /// An IWA or Snappy framing invariant was violated.
    InvalidIwa(&'static str),
    /// A read ran off the end of an expected byte range.
    Truncated(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::NotAZipArchive => write!(f, "file does not look like a zip archive"),
            Self::UnsupportedDocumentType(path) => {
                write!(f, "unsupported iWork document type: {path}")
            }
            Self::UnsupportedIwaChunkType(kind) => {
                write!(f, "unsupported iwa chunk type: {kind}")
            }
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
            Self::InvalidCompressedEntry { path } => {
                write!(f, "entry {path} could not be decompressed")
            }
            Self::MissingEntry(path) => write!(f, "missing entry: {path}"),
            Self::InvalidUtf8(error) => write!(f, "invalid UTF-8: {error}"),
            Self::InvalidPlist(message) => write!(f, "invalid plist: {message}"),
            Self::InvalidIwa(message) => write!(f, "invalid iwa: {message}"),
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
