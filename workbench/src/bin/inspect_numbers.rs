use std::fmt::Write as _;

use iwork::iwa::IwaArchive;
use iwork::numbers::{self, message_type_name};
use iwork::package::Package;
use protorev::{Message, dump_message};

/// Recursion depth for the protobuf dumps.
const DUMP_DEPTH: usize = 6;
/// Cap on per-archive object payload dumps so composite archives stay readable.
const MAX_OBJECT_DUMPS: usize = 6;

fn main() -> Result<(), iwork::Error> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .unwrap_or_else(|| "examples/numbers/personal_budget.numbers".to_owned());
    let filter = args.next();
    let document = numbers::Document::open(&path)?;
    let spreadsheet = document.spreadsheet()?;
    let package = Package::open(&path)?;

    if filter.is_none() {
        inspect_archive("Document", spreadsheet.document());
        inspect_archive("DocumentMetadata", spreadsheet.document_metadata());
        inspect_archive("Metadata", spreadsheet.metadata());
        inspect_entry(&package, "Index/ObjectContainer.iwa")?;
        inspect_entry(&package, "Index/CalculationEngine.iwa")?;
        inspect_entry(&package, "Index/ViewState.iwa")?;
        inspect_archive("Stylesheet", spreadsheet.stylesheet());
    }

    for archive in spreadsheet.table_archives() {
        if filter
            .as_deref()
            .is_some_and(|pattern| !archive.path().contains(pattern))
        {
            continue;
        }
        inspect_archive(archive.path(), archive.archive());
    }

    Ok(())
}

fn inspect_entry(package: &Package, path: &str) -> Result<(), iwork::Error> {
    let archive = IwaArchive::decode(package.entry_bytes(path)?)?;
    inspect_archive(path, &archive);
    Ok(())
}

/// Summarizes the IWA framing, then hands each object's protobuf payload to
/// `protorev` for the field-level dump.
fn inspect_archive(label: &str, archive: &IwaArchive) {
    println!("== {label} ==");
    println!(
        "descriptor: root={:?} kind={:?} body_hint={:?} refs={}",
        archive.descriptor().root_object_id,
        archive.descriptor().kind_hint,
        archive.descriptor().body_hint,
        archive.descriptor().object_references.len()
    );
    println!(
        "leading object refs: {:?}",
        archive.leading_object_references()
    );

    let objects = archive.objects();
    println!("objects: {}", objects.len());
    for (index, object) in objects.iter().enumerate() {
        let type_name = object
            .message_type
            .and_then(message_type_name)
            .unwrap_or("?");
        println!(
            "  object {index}: id={:?} type={:?} ({type_name}) payload_len={}",
            object.identifier,
            object.message_type,
            object.payload.len()
        );
        if index >= MAX_OBJECT_DUMPS {
            continue;
        }
        match Message::decode(&object.payload) {
            Ok(message) => print!("{}", indent(&dump_message(&message, DUMP_DEPTH))),
            Err(error) => println!("    <decode error: {error}>"),
        }
    }
    if objects.len() > MAX_OBJECT_DUMPS {
        println!("  ... {} more object payloads not dumped", objects.len() - MAX_OBJECT_DUMPS);
    }
    println!();
}

/// Indents a multi-line dump so it nests under its object header.
fn indent(text: &str) -> String {
    text.lines().fold(String::new(), |mut acc, line| {
        let _ = writeln!(acc, "    {line}");
        acc
    })
}
