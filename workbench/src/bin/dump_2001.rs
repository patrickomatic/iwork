//! Hex-dumps the largest type-2001 TSWP object from a Pages/Keynote file.
use iwork::Error;
use iwork::iwa::IwaArchive;
use iwork::package::Package;

fn main() -> Result<(), Error> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_2001 <file> [limit]");
    let limit: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(512);
    let package = Package::open(&path)?;
    let bytes = package.entry_bytes("Index/Document.iwa")?;
    let archive = IwaArchive::decode(bytes)?;
    let largest = archive
        .objects()
        .into_iter()
        .filter(|o| o.message_type == Some(2001))
        .max_by_key(|o| o.payload.len())
        .expect("no type-2001 object");
    eprintln!(
        "id={:?} payload_len={}",
        largest.identifier,
        largest.payload.len()
    );
    hexdump(&largest.payload, limit);
    Ok(())
}

fn hexdump(data: &[u8], limit: usize) {
    let data = &data[..data.len().min(limit)];
    for (i, chunk) in data.chunks(16).enumerate() {
        print!("{:06x}  ", i * 16_usize);
        for b in chunk {
            print!("{b:02x} ");
        }
        for _ in chunk.len()..16 {
            print!("   ");
        }
        print!(" |");
        for b in chunk {
            print!(
                "{}",
                if b.is_ascii_graphic() || *b == b' ' {
                    *b as char
                } else {
                    '.'
                }
            );
        }
        println!("|");
    }
}
