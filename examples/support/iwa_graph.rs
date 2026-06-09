#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use iwork::{Error, IwaArchive, Package, ProtoMessage, ProtoValue};

const MAX_PROTO_DEPTH: usize = 12;
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
    chunks: Vec<ChunkSummary>,
    body_len: usize,
    top_level_fields: BTreeMap<u32, usize>,
    shape_paths: BTreeMap<String, usize>,
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
                    top_level_fields: BTreeMap::new(),
                    shape_paths: BTreeMap::new(),
                    strings: archive.ascii_strings(4),
                    decode_error: None,
                };

                summary.strings.sort();
                summary.strings.dedup();
                summary.strings.truncate(MAX_STRINGS_PER_ARCHIVE);

                if let Some(messages) = collect_body_stream(
                    archive.body(),
                    archive.leading_object_references_len(),
                    &mut summary.top_level_fields,
                ) {
                    for (index, message) in messages.iter().enumerate() {
                        collect_message_shape(
                            message,
                            &format!("$stream[{index}]"),
                            0,
                            &mut summary.shape_paths,
                        );
                    }
                } else {
                    match ProtoMessage::decode(archive.body()) {
                        Ok(message) => {
                            for field in message.fields() {
                                *summary.top_level_fields.entry(field.number).or_insert(0) += 1;
                            }
                            collect_message_shape(&message, "$", 0, &mut summary.shape_paths);
                        }
                        Err(error) => {
                            summary.decode_error = Some(error.to_string());
                        }
                    }
                }

                summary
            }
            Err(error) => Self {
                root_object_id: None,
                kind_hint: None,
                body_hint: None,
                object_references: Vec::new(),
                leading_object_references: Vec::new(),
                chunks: Vec::new(),
                body_len: 0,
                top_level_fields: BTreeMap::new(),
                shape_paths: BTreeMap::new(),
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

        let _ = writeln!(out, "top_level_fields:");
        for (field, count) in &self.top_level_fields {
            let _ = writeln!(out, "  f{field}: {count}");
        }

        let _ = writeln!(out, "shape:");
        for (shape, count) in &self.shape_paths {
            let _ = writeln!(out, "  {shape} x{count}");
        }

        let _ = writeln!(out, "strings:");
        for value in &self.strings {
            let _ = writeln!(out, "  {value:?}");
        }
        let _ = writeln!(out);
    }
}

fn collect_body_stream(
    bytes: &[u8],
    start: usize,
    top_level_fields: &mut BTreeMap<u32, usize>,
) -> Option<Vec<ProtoMessage>> {
    let mut cursor = start;
    let mut messages = Vec::new();

    while cursor < bytes.len() {
        let tag = read_varint(bytes, &mut cursor).ok()?;
        if tag == 0 {
            return None;
        }
        let field_number = u32::try_from(tag >> 3).ok()?;
        let wire_type = tag & 0x07;
        *top_level_fields.entry(field_number).or_insert(0) += 1;

        match wire_type {
            0 => {
                let _ = read_varint(bytes, &mut cursor).ok()?;
            }
            1 => {
                cursor = cursor.checked_add(8)?;
                if cursor > bytes.len() {
                    return None;
                }
            }
            2 => {
                let len = usize::try_from(read_varint(bytes, &mut cursor).ok()?).ok()?;
                let end = cursor.checked_add(len)?;
                let value = bytes.get(cursor..end)?;
                cursor = end;
                if let Ok(message) = ProtoMessage::decode(value) {
                    messages.push(message);
                }
            }
            5 => {
                cursor = cursor.checked_add(4)?;
                if cursor > bytes.len() {
                    return None;
                }
            }
            _ => return None,
        }
    }

    Some(messages)
}

fn collect_message_shape(
    message: &ProtoMessage,
    path: &str,
    depth: usize,
    shapes: &mut BTreeMap<String, usize>,
) {
    if depth >= MAX_PROTO_DEPTH {
        return;
    }

    for field in message.fields() {
        let child_path = format!("{path}.{}", field.number);
        let descriptor = match &field.value {
            ProtoValue::Varint(_) => "varint".to_owned(),
            ProtoValue::Fixed32(_) => "fixed32".to_owned(),
            ProtoValue::Fixed64(_) => "fixed64".to_owned(),
            ProtoValue::LengthDelimited(bytes) => {
                let len_bucket = length_bucket(bytes.len());
                match ProtoMessage::decode(bytes) {
                    Ok(nested) => {
                        *shapes
                            .entry(format!("{child_path}:message:{len_bucket}"))
                            .or_insert(0) += 1;
                        collect_message_shape(&nested, &child_path, depth + 1, shapes);
                        continue;
                    }
                    Err(_) => format!("bytes:{len_bucket}"),
                }
            }
        };
        *shapes
            .entry(format!("{child_path}:{descriptor}"))
            .or_insert(0) += 1;
    }
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, ()> {
    let mut shift = 0u32;
    let mut value = 0u64;

    loop {
        if shift >= 64 {
            return Err(());
        }

        let byte = *bytes.get(*cursor).ok_or(())?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;

        if byte & 0x80 == 0 {
            return Ok(value);
        }

        shift += 7;
    }
}

fn length_bucket(len: usize) -> &'static str {
    match len {
        0 => "0",
        1..=4 => "1..4",
        5..=16 => "5..16",
        17..=64 => "17..64",
        65..=256 => "65..256",
        257..=1024 => "257..1024",
        _ => "1025+",
    }
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
        "top_level_fields",
        &left.top_level_fields,
        &right.top_level_fields,
        &mut section,
    );
    diff_map("shape", &left.shape_paths, &right.shape_paths, &mut section);
    diff_vec("strings", &left.strings, &right.strings, &mut section);

    if !section.is_empty() {
        let _ = writeln!(out, "== {path} ==");
        out.push_str(&section);
        let _ = writeln!(out);
    }
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
