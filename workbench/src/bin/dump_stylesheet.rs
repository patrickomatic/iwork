//! Dumps all named-style entries from the type-401 `DocumentStylesheet` object.
//!
//! Usage: `dump_stylesheet` <file.pages|file.key> [filter]
//!
//! Each output line: `<object_id>  <style_key>`.
//! If `filter` is given, only lines whose `style_key` contains that substring are
//! printed (case-insensitive).
use iwork::Error;
use iwork::iwa::IwaArchive;
use iwork::package::Package;
use iwork::protobuf::ProtoMessage;

const STYLESHEET_TYPE: u64 = 401;
const STYLESHEET_ENTRY: &str = "Index/DocumentStylesheet.iwa";

fn main() -> Result<(), Error> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_stylesheet <file> [filter]");
    let filter = std::env::args().nth(2).unwrap_or_default().to_lowercase();

    let package = Package::open(&path)?;
    let bytes = package.entry_bytes(STYLESHEET_ENTRY)?;
    let archive = IwaArchive::decode(bytes)?;

    let stylesheet = archive
        .objects()
        .into_iter()
        .find(|o| o.message_type == Some(STYLESHEET_TYPE))
        .expect("no type-401 object in DocumentStylesheet.iwa");

    let msg = ProtoMessage::decode(&stylesheet.payload).expect("failed to decode 401 payload");

    // field 2 is repeated; each entry has:
    //   field 2.1 = style key (UTF-8)
    //   field 2.2.1 = object ID (uint64)
    let mut entries: Vec<(u64, String)> = Vec::new();

    for entry in msg.fields_by_number(2) {
        let Some(bytes_2) = entry.value.as_bytes() else {
            continue;
        };
        let Ok(inner) = ProtoMessage::decode(bytes_2) else {
            continue;
        };

        let key_bytes = inner
            .field(1)
            .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec));
        let id = inner
            .field(2)
            .and_then(|f| f.value.as_bytes().map(<[u8]>::to_vec))
            .and_then(|b| ProtoMessage::decode(&b).ok())
            .and_then(|m| m.field(1).and_then(|f| f.value.as_varint()));

        if let (Some(key_bytes), Some(id)) = (key_bytes, id)
            && let Ok(key) = String::from_utf8(key_bytes)
        {
            entries.push((id, key));
        }
    }

    entries.sort_by_key(|(id, _)| *id);

    for (id, key) in &entries {
        if filter.is_empty() || key.to_lowercase().contains(&filter) {
            println!("{id:>10}  {key}");
        }
    }

    let shown = if filter.is_empty() {
        entries.len()
    } else {
        entries
            .iter()
            .filter(|(_, k)| k.to_lowercase().contains(&filter))
            .count()
    };
    eprintln!("{} total named styles, {} shown", entries.len(), shown);

    Ok(())
}
