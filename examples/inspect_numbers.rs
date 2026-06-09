use iwork::ProtoValue;
use iwork::numbers;

fn main() -> Result<(), iwork::Error> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .unwrap_or_else(|| "examples/numbers/personal_budget.numbers".to_owned());
    let filter = args.next();
    let document = numbers::Document::open(&path)?;
    let spreadsheet = document.spreadsheet()?;
    let package = document.package();

    if filter.is_none() {
        inspect_archive("Document", spreadsheet.document())?;
        inspect_archive("DocumentMetadata", spreadsheet.document_metadata())?;
        inspect_archive("Metadata", spreadsheet.metadata())?;
        inspect_entry(package, "Index/ObjectContainer.iwa")?;
        inspect_entry(package, "Index/CalculationEngine.iwa")?;
        inspect_entry(package, "Index/ViewState.iwa")?;
        inspect_archive("Stylesheet", spreadsheet.stylesheet())?;
    }

    for archive in spreadsheet.table_archives() {
        if filter
            .as_deref()
            .is_some_and(|pattern| !archive.path().contains(pattern))
        {
            continue;
        }
        println!("== {} ==", archive.path());
        inspect_archive(archive.path(), archive.archive())?;
    }

    Ok(())
}

fn inspect_entry(package: &iwork::Package, path: &str) -> Result<(), iwork::Error> {
    let archive = iwork::IwaArchive::decode(package.entry_bytes(path)?)?;
    inspect_archive(path, &archive)
}

fn inspect_archive(label: &str, archive: &iwork::IwaArchive) -> Result<(), iwork::Error> {
    println!("== {label} ==");
    println!(
        "descriptor: root={:?} kind={:?} body_hint={:?} refs={}",
        archive.descriptor().root_object_id,
        archive.descriptor().kind_hint,
        archive.descriptor().body_hint,
        archive.descriptor().object_references.len()
    );

    let header = archive.header().decode_message()?;
    println!("header:");
    print_message(&header, 1);

    println!(
        "leading object refs: {:?}",
        archive.leading_object_references()
    );
    println!("body preview:");

    if let Ok(message) = iwork::ProtoMessage::decode(archive.body()) {
        println!("  body-as-message:");
        print_message(&message, 2);
    }

    let body = archive.body();
    let mut cursor = archive.leading_object_references_len();
    let mut shown = 0usize;
    while cursor < body.len() && shown < 4 {
        let start = cursor;
        let Ok(tag) = read_varint(body, &mut cursor) else {
            break;
        };
        if (tag & 0x07) != 2 {
            break;
        }
        let Ok(len) = read_varint(body, &mut cursor) else {
            break;
        };
        let Ok(len) = usize::try_from(len) else {
            break;
        };
        let Some(chunk) = body.get(cursor..cursor + len) else {
            break;
        };
        cursor += len;

        println!("  message {} at body offset {}", shown + 1, start);
        match iwork::ProtoMessage::decode(chunk) {
            Ok(message) => print_message(&message, 2),
            Err(error) => println!("    <decode error: {error}>"),
        }
        shown += 1;
    }
    println!();
    Ok(())
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, ()> {
    let mut shift = 0u32;
    let mut value = 0u64;

    loop {
        if shift >= 64 {
            return Err(());
        }
        let Some(byte) = bytes.get(*cursor).copied() else {
            return Err(());
        };
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}

fn print_message(message: &iwork::ProtoMessage, indent: usize) {
    let prefix = " ".repeat(indent * 2);
    for field in message.fields() {
        match &field.value {
            ProtoValue::Varint(value) => {
                println!("{prefix}field {}: varint {value}", field.number);
            }
            ProtoValue::Fixed32(value) => {
                println!("{prefix}field {}: fixed32 {value}", field.number);
            }
            ProtoValue::Fixed64(value) => {
                println!("{prefix}field {}: fixed64 {value}", field.number);
            }
            ProtoValue::LengthDelimited(bytes) => {
                let ascii = if bytes
                    .iter()
                    .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
                {
                    std::str::from_utf8(bytes).ok()
                } else {
                    None
                };
                println!(
                    "{prefix}field {}: bytes len={}{}",
                    field.number,
                    bytes.len(),
                    ascii.map_or(String::new(), |text| format!(" ascii={text:?}"))
                );
                if let Ok(nested) = iwork::ProtoMessage::decode(bytes) {
                    print_message(&nested, indent + 1);
                }
            }
        }
    }
}
