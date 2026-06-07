use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    NotAZipArchive,
    UnsupportedDocumentType(String),
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
            Self::UnsupportedDocumentType(path) => {
                write!(f, "unsupported iWork document type: {path}")
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
