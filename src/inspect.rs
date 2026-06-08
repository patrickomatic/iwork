use std::collections::BTreeMap;

use crate::{DocumentKind, PackageSupport, PropertiesPlist};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InspectionReport {
    pub path: String,
    pub kind: DocumentKind,
    pub support: PackageSupport,
    pub properties: PropertiesPlist,
    pub entry_count: usize,
    pub iwa_count: usize,
    pub style_keyword_hits: BTreeMap<String, usize>,
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
