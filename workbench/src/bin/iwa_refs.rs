//! Cross-object reference-graph explorer for iWork packages.
//!
//! This is the object-graph complement to `iwa_corpus` (which inspects what is
//! *inside* objects of one type). Reference edges are recovered structurally:
//! object identifiers are large unique integers, so a payload varint equal to
//! another object's identifier is a reliable reference edge — no schema needed.
//! This is the technique that named the `Sheet → TableInfo → TableModel` chain.
//!
//! ```text
//! cargo run --example iwa_refs -- types <file.numbers>...
//! cargo run --example iwa_refs -- edges <type> <file.numbers>...
//! cargo run --example iwa_refs -- refs  <object-id> <file.numbers>
//! ```
//!
//! `types` prints the object-type histogram; `edges` aggregates, for every
//! object of one message type, the types it references (counted per referenced
//! object); and `refs` lists which objects reference a given object id.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

use iwork::numbers::message_type_name;
use iwork::iwa::IwaArchive;
use iwork::package::Package;

struct ObjectInfo {
    id: u64,
    message_type: u64,
    payload: Vec<u8>,
}

/// All objects in one package, plus an id → message-type index.
struct PackageObjects {
    id_type: BTreeMap<u64, u64>,
    objects: Vec<ObjectInfo>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first() else {
        print_usage();
        return Ok(());
    };
    let rest = &argv[1..];

    match command.as_str() {
        "types" => types_command(rest),
        "edges" => edges_command(rest),
        "refs" => refs_command(rest),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(err(format!("unknown command {other:?}"))),
    }
}

fn types_command(files: &[String]) -> Result<(), Box<dyn Error>> {
    if files.is_empty() {
        return Err(err("missing <file.numbers>..."));
    }

    let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
    for file in files {
        for object in load(file)?.objects {
            *counts.entry(object.message_type).or_insert(0) += 1;
        }
    }

    for (message_type, count) in &counts {
        println!("{} = {count}", label(*message_type));
    }
    Ok(())
}

fn edges_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let message_type = parse_u64(args.first(), "<type>")?;
    let files = &args[1..];
    if files.is_empty() {
        return Err(err("missing <file.numbers>..."));
    }

    let mut source_objects = 0usize;
    let mut targets: BTreeMap<u64, usize> = BTreeMap::new();
    for file in files {
        let package = load(file)?;
        for object in &package.objects {
            if object.message_type != message_type {
                continue;
            }
            source_objects += 1;
            for referenced in referenced_ids(object, &package.id_type) {
                if let Some(target_type) = package.id_type.get(&referenced) {
                    *targets.entry(*target_type).or_insert(0) += 1;
                }
            }
        }
    }

    println!("{}: {source_objects} object(s)", label(message_type));
    for (target_type, count) in &targets {
        println!("  -> {} x{count}", label(*target_type));
    }
    Ok(())
}

fn refs_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let target_id = parse_u64(args.first(), "<object-id>")?;
    let file = args.get(1).ok_or_else(|| err("missing <file.numbers>"))?;
    let package = load(file)?;

    let target_type = package
        .id_type
        .get(&target_id)
        .map_or_else(|| "unknown".to_owned(), |t| label(*t));
    println!("object {target_id} ({target_type}) is referenced by:");
    for object in &package.objects {
        if object.id != target_id && referenced_ids(object, &package.id_type).contains(&target_id) {
            println!("  {} id={}", label(object.message_type), object.id);
        }
    }
    Ok(())
}

/// Loads every object in a package and an id → message-type index.
fn load(file: &str) -> Result<PackageObjects, Box<dyn Error>> {
    let package = Package::open(file)?;
    let mut id_type = BTreeMap::new();
    let mut objects = Vec::new();

    for entry in package.entries() {
        if !Path::new(&entry.path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("iwa"))
        {
            continue;
        }
        let Ok(archive) = IwaArchive::decode(package.entry_bytes(&entry.path)?) else {
            continue;
        };
        for object in archive.objects() {
            if let (Some(id), Some(message_type)) = (object.identifier, object.message_type) {
                id_type.insert(id, message_type);
                objects.push(ObjectInfo {
                    id,
                    message_type,
                    payload: object.payload,
                });
            }
        }
    }

    Ok(PackageObjects { id_type, objects })
}

/// Returns the distinct known object ids referenced by `object` (excluding
/// itself), recovered by scanning the payload for varints that match an id.
fn referenced_ids(object: &ObjectInfo, id_type: &BTreeMap<u64, u64>) -> Vec<u64> {
    let mut referenced = Vec::new();
    for start in 0..object.payload.len() {
        let mut cursor = start;
        let Some(value) = read_varint(&object.payload, &mut cursor) else {
            continue;
        };
        if value != object.id && id_type.contains_key(&value) && !referenced.contains(&value) {
            referenced.push(value);
        }
    }
    referenced
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut shift = 0u32;
    let mut value = 0u64;
    loop {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

fn label(message_type: u64) -> String {
    let name = message_type_name(message_type).unwrap_or("?");
    format!("{message_type} ({name})")
}

fn parse_u64(arg: Option<&String>, name: &str) -> Result<u64, Box<dyn Error>> {
    let raw = arg.ok_or_else(|| err(format!("missing {name}")))?;
    raw.parse::<u64>()
        .map_err(|_| err(format!("invalid {name}: {raw:?}")))
}

fn err(message: impl Into<String>) -> Box<dyn Error> {
    Box::<dyn Error>::from(message.into())
}

fn print_usage() {
    println!("iwa_refs: cross-object reference graph for iWork packages");
    println!();
    println!("usage:");
    println!("  iwa_refs types <file.numbers>...");
    println!("  iwa_refs edges <type> <file.numbers>...");
    println!("  iwa_refs refs  <object-id> <file.numbers>");
}
