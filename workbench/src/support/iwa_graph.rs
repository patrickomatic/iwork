#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use iwork::numbers::message_type_name;
use iwork::iwa::IwaArchive;
use iwork::package::Package;
use iwork::Error;
use protorev::{Corpus, Message};

/// Recursion depth for protorev corpus aggregation over object payloads.
const CORPUS_DEPTH: usize = 8;
const MAX_STRINGS_PER_ARCHIVE: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageGraph {
    entries: Vec<EntrySummary>,
    archives: BTreeMap<String, ArchiveSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntrySummary {
    path: String,
    compression_method: u16,
    compressed_size: u32,
    uncompressed_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchiveSummary {
    root_object_id: Option<u64>,
    kind_hint: Option<u64>,
    body_hint: Option<u64>,
    object_references: Vec<ObjectReferenceSummary>,
    leading_object_references: Vec<u64>,
    objects: Vec<ObjectSummary>,
    /// Primary-message payload of each object, fed to `protorev` for shape.
    object_payloads: Vec<Vec<u8>>,
    chunks: Vec<ChunkSummary>,
    body_len: usize,
    strings: Vec<String>,
    decode_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectReferenceSummary {
    object_id: Option<u64>,
    kind_hint: Option<u64>,
    state_hint: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectSummary {
    identifier: Option<u64>,
    message_type: Option<u64>,
    type_name: Option<&'static str>,
    payload_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChunkSummary {
    kind: u8,
    compressed_len: usize,
    decompressed_len: usize,
}

pub fn dump_package(path: &Path) -> Result<String, Error> {
    let graph = PackageGraph::from_path(path)?;
    Ok(graph.dump(path))
}

pub fn diff_packages(left_path: &Path, right_path: &Path) -> Result<String, Error> {
    let left = PackageGraph::from_path(left_path)?;
    let right = PackageGraph::from_path(right_path)?;
    Ok(diff_graphs(left_path, &left, right_path, &right))
}

impl PackageGraph {
    fn from_path(path: &Path) -> Result<Self, Error> {
        let package = Package::open(path)?;
        let mut entries = package
            .entries()
            .iter()
            .map(|entry| EntrySummary {
                path: entry.path.clone(),
                compression_method: entry.compression_method,
                compressed_size: entry.compressed_size,
                uncompressed_size: entry.uncompressed_size,
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.path.cmp(&right.path));

        let mut archives = BTreeMap::new();
        for entry in &entries {
            if !Path::new(&entry.path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("iwa"))
            {
                continue;
            }
            let bytes = package.entry_bytes(&entry.path)?;
            archives.insert(entry.path.clone(), ArchiveSummary::from_bytes(bytes));
        }

        Ok(Self { entries, archives })
    }

    fn dump(&self, path: &Path) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "package: {}", path.display());
        let _ = writeln!(out, "entries: {}", self.entries.len());
        let _ = writeln!(out, "iwa_archives: {}", self.archives.len());
        let _ = writeln!(out);

        let _ = writeln!(out, "== entries ==");
        for entry in &self.entries {
            let _ = writeln!(
                out,
                "{} method={} compressed={} uncompressed={}",
                entry.path,
                entry.compression_method,
                entry.compressed_size,
                entry.uncompressed_size
            );
        }
        let _ = writeln!(out);

        for (path, archive) in &self.archives {
            archive.dump(path, &mut out);
        }

        out
    }
}

impl ArchiveSummary {
    fn from_bytes(bytes: &[u8]) -> Self {
        match IwaArchive::decode(bytes) {
            Ok(archive) => {
                let decoded_objects = archive.objects();
                let mut summary = Self {
                    root_object_id: archive.descriptor().root_object_id,
                    kind_hint: archive.descriptor().kind_hint,
                    body_hint: archive.descriptor().body_hint,
                    object_references: archive
                        .descriptor()
                        .object_references
                        .iter()
                        .map(|reference| ObjectReferenceSummary {
                            object_id: reference.object_id,
                            kind_hint: reference.kind_hint,
                            state_hint: reference.state_hint,
                        })
                        .collect(),
                    leading_object_references: archive.leading_object_references(),
                    objects: decoded_objects
                        .iter()
                        .map(|object| ObjectSummary {
                            identifier: object.identifier,
                            message_type: object.message_type,
                            type_name: object.message_type.and_then(message_type_name),
                            payload_len: object.payload.len(),
                        })
                        .collect(),
                    object_payloads: decoded_objects
                        .iter()
                        .map(|object| object.payload.clone())
                        .collect(),
                    chunks: archive
                        .chunks()
                        .iter()
                        .map(|chunk| ChunkSummary {
                            kind: chunk.kind,
                            compressed_len: chunk.compressed_len,
                            decompressed_len: chunk.decompressed_len,
                        })
                        .collect(),
                    body_len: archive.body().len(),
                    strings: archive.ascii_strings(4),
                    decode_error: None,
                };

                summary.strings.sort();
                summary.strings.dedup();
                summary.strings.truncate(MAX_STRINGS_PER_ARCHIVE);
                summary
            }
            Err(error) => Self {
                root_object_id: None,
                kind_hint: None,
                body_hint: None,
                object_references: Vec::new(),
                leading_object_references: Vec::new(),
                objects: Vec::new(),
                object_payloads: Vec::new(),
                chunks: Vec::new(),
                body_len: 0,
                strings: Vec::new(),
                decode_error: Some(error.to_string()),
            },
        }
    }

    fn dump(&self, path: &str, out: &mut String) {
        let _ = writeln!(out, "== {path} ==");
        let _ = writeln!(
            out,
            "descriptor root={:?} kind={:?} body_hint={:?} body_len={}",
            self.root_object_id, self.kind_hint, self.body_hint, self.body_len
        );

        if let Some(error) = &self.decode_error {
            let _ = writeln!(out, "decode_error={error}");
            let _ = writeln!(out);
            return;
        }

        let _ = writeln!(out, "chunks:");
        for chunk in &self.chunks {
            let _ = writeln!(
                out,
                "  kind={} compressed={} decompressed={}",
                chunk.kind, chunk.compressed_len, chunk.decompressed_len
            );
        }

        let _ = writeln!(out, "descriptor_refs:");
        for reference in &self.object_references {
            let _ = writeln!(
                out,
                "  object={:?} kind={:?} state={:?}",
                reference.object_id, reference.kind_hint, reference.state_hint
            );
        }
        let _ = writeln!(out, "leading_refs: {:?}", self.leading_object_references);

        let _ = writeln!(out, "objects:");
        for object in &self.objects {
            let _ = writeln!(
                out,
                "  id={:?} type={:?} ({}) payload_len={}",
                object.identifier,
                object.message_type,
                object.type_name.unwrap_or("?"),
                object.payload_len
            );
        }

        // Protobuf field shape across this archive's object payloads, via the
        // protorev workbench (the iwork tooling no longer hand-rolls this).
        let corpus = build_corpus(&self.object_payloads);
        let _ = writeln!(out, "proto shape (protorev):");
        for line in corpus.summary().lines() {
            let _ = writeln!(out, "  {line}");
        }

        let _ = writeln!(out, "strings:");
        for value in &self.strings {
            let _ = writeln!(out, "  {value:?}");
        }
        let _ = writeln!(out);
    }
}

/// Builds a protorev corpus from the protobuf payloads that decode cleanly.
fn build_corpus(object_payloads: &[Vec<u8>]) -> Corpus {
    let messages = object_payloads
        .iter()
        .filter_map(|payload| Message::decode(payload).ok())
        .collect::<Vec<_>>();
    Corpus::from_messages(&messages, CORPUS_DEPTH)
}

fn diff_graphs(
    left_path: &Path,
    left: &PackageGraph,
    right_path: &Path,
    right: &PackageGraph,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "left: {}", left_path.display());
    let _ = writeln!(out, "right: {}", right_path.display());
    let _ = writeln!(out);

    diff_entry_sets(left, right, &mut out);
    diff_archives(left, right, &mut out);

    out
}

fn diff_entry_sets(left: &PackageGraph, right: &PackageGraph, out: &mut String) {
    let left_entries = left
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let right_entries = right
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();

    let _ = writeln!(out, "== entry set ==");
    for path in left_entries.difference(&right_entries) {
        let _ = writeln!(out, "- {path}");
    }
    for path in right_entries.difference(&left_entries) {
        let _ = writeln!(out, "+ {path}");
    }
    let _ = writeln!(out);
}

fn diff_archives(left: &PackageGraph, right: &PackageGraph, out: &mut String) {
    let paths = left
        .archives
        .keys()
        .chain(right.archives.keys())
        .collect::<BTreeSet<_>>();

    for path in paths {
        let left_archive = left.archives.get(path);
        let right_archive = right.archives.get(path);
        match (left_archive, right_archive) {
            (Some(left_archive), Some(right_archive)) => {
                diff_archive(path, left_archive, right_archive, out);
            }
            (Some(_), None) => {
                let _ = writeln!(out, "== {path} ==");
                let _ = writeln!(out, "only in left");
                let _ = writeln!(out);
            }
            (None, Some(_)) => {
                let _ = writeln!(out, "== {path} ==");
                let _ = writeln!(out, "only in right");
                let _ = writeln!(out);
            }
            (None, None) => {}
        }
    }
}

fn diff_archive(path: &str, left: &ArchiveSummary, right: &ArchiveSummary, out: &mut String) {
    let mut section = String::new();

    if left.kind_hint != right.kind_hint {
        let _ = writeln!(
            section,
            "kind_hint: {:?} -> {:?}",
            left.kind_hint, right.kind_hint
        );
    }
    if left.body_len != right.body_len {
        let _ = writeln!(section, "body_len: {} -> {}", left.body_len, right.body_len);
    }
    if left.decode_error != right.decode_error {
        let _ = writeln!(
            section,
            "decode_error: {:?} -> {:?}",
            left.decode_error, right.decode_error
        );
    }
    diff_map(
        "object_type_counts",
        &object_type_counts(&left.objects),
        &object_type_counts(&right.objects),
        &mut section,
    );
    if left.object_payloads != right.object_payloads {
        let corpus_diff = Corpus::diff(
            &build_corpus(&left.object_payloads),
            &build_corpus(&right.object_payloads),
        );
        let _ = writeln!(section, "proto shape diff (protorev):");
        for line in corpus_diff.lines() {
            let _ = writeln!(section, "  {line}");
        }
    }
    diff_vec("strings", &left.strings, &right.strings, &mut section);

    if !section.is_empty() {
        let _ = writeln!(out, "== {path} ==");
        out.push_str(&section);
        let _ = writeln!(out);
    }
}

/// Counts objects by their message type, labeling known types for readability.
fn object_type_counts(objects: &[ObjectSummary]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for object in objects {
        let label = match (object.message_type, object.type_name) {
            (Some(message_type), Some(name)) => format!("{message_type} ({name})"),
            (Some(message_type), None) => message_type.to_string(),
            (None, _) => "none".to_owned(),
        };
        *counts.entry(label).or_insert(0) += 1;
    }
    counts
}

fn diff_map<K: Ord + std::fmt::Debug, V: Eq + std::fmt::Debug>(
    label: &str,
    left: &BTreeMap<K, V>,
    right: &BTreeMap<K, V>,
    out: &mut String,
) {
    if left == right {
        return;
    }

    let _ = writeln!(out, "{label}:");
    for key in left
        .keys()
        .collect::<BTreeSet<_>>()
        .union(&right.keys().collect::<BTreeSet<_>>())
    {
        match (left.get(*key), right.get(*key)) {
            (Some(left_value), Some(right_value)) if left_value == right_value => {}
            (Some(left_value), Some(right_value)) => {
                let _ = writeln!(out, "  {key:?}: {left_value:?} -> {right_value:?}");
            }
            (Some(left_value), None) => {
                let _ = writeln!(out, "  - {key:?}: {left_value:?}");
            }
            (None, Some(right_value)) => {
                let _ = writeln!(out, "  + {key:?}: {right_value:?}");
            }
            (None, None) => {}
        }
    }
}

fn diff_vec(label: &str, left: &[String], right: &[String], out: &mut String) {
    let left_set = left.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let right_set = right.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if left_set == right_set {
        return;
    }

    let _ = writeln!(out, "{label}:");
    for value in left_set.difference(&right_set) {
        let _ = writeln!(out, "  - {value:?}");
    }
    for value in right_set.difference(&left_set) {
        let _ = writeln!(out, "  + {value:?}");
    }
}
