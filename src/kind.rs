use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DocumentKind {
    Numbers,
    Pages,
    Keynote,
    #[default]
    Unknown,
}

impl DocumentKind {
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        match path
            .as_ref()
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("numbers") => Self::Numbers,
            Some("pages") => Self::Pages,
            Some("key") => Self::Keynote,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Numbers => "numbers",
            Self::Pages => "pages",
            Self::Keynote => "keynote",
            Self::Unknown => "unknown",
        }
    }
}
