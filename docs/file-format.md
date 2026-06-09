# iWork File Format Notes

This document records the file-format assumptions the crate currently depends on. It is not intended to be a complete description of Apple iWork internals; it describes the parts we have verified well enough to build against today.

## Container Format

All supported iWork documents in this repository are ZIP archives.

The crate currently recognizes these package-layout variants:

- `SupportedDirectIndexEntries`:
  ZIP archive with document members directly under `Index/...`
- `UnsupportedLegacyIndexZip`:
  legacy-style package containing `Index.zip`
- `UnsupportedUnknownLayout`:
  ZIP archive that does not match either known layout

Only `SupportedDirectIndexEntries` is currently supported by the reader APIs
and test fixtures in this repository.

The crate currently relies on these ZIP-level properties:

- the file starts with a local file header signature `0x04034B50`
- the archive contains a standard end-of-central-directory record
- central directory records use the standard signature `0x02014B50`
- local file headers use the standard signature `0x04034B50`
- entry names are UTF-8
- entry data is read from stored entries with compression method `0`
- entry data is read from deflated entries with compression method `8`

Other ZIP compression methods still return `Error::UnsupportedCompression`.

## Package Layout

The package layout we currently rely on is small:

- `Metadata/Properties.plist`
- `Index/DocumentStylesheet.iwa`
- `Index/Document.iwa`
- `Index/DocumentMetadata.iwa`
- `Index/Metadata.iwa`
- `Index/ObjectContainer.iwa`
- `Index/CalculationEngine.iwa`
- `Index/ViewState.iwa`
- `Index/Tables/*.iwa` for Numbers table data

`Metadata/Properties.plist` is used for stable metadata exposed by `PropertiesPlist`.

`Index/DocumentStylesheet.iwa` is currently treated as an opaque byte payload. The inspection path only scans it for simple keyword occurrences such as `bold`, `italic`, `underline`, `strikethrough`, and `font`.

Numbers spreadsheet reading additionally decodes the table `Tile` and
`DataList` archives described below.

## Document Type Detection

Document kind is inferred from the filename extension:

- `.numbers` => Numbers
- `.pages` => Pages
- `.key` => Keynote

This is a convenience classification for the public API, not a guarantee derived from package internals.

## `Properties.plist`

The crate supports both XML plist and binary plist encodings for `Metadata/Properties.plist`.

The XML parser is intentionally narrow. It expects:

- a top-level `<dict>`
- `<key>` entries paired with either `<string>`, `<true/>`, or `<false/>`

The binary plist parser is also intentionally narrow. It currently supports only the object types needed by fixture documents:

- ASCII strings
- UTF-16BE strings
- booleans
- dictionaries

From those plist payloads we currently surface these keys when present:

- `documentUUID`
- `fileFormatVersion`
- `isMultiPage`
- `revision`
- `stableDocumentUUID`
- `versionUUID`

Unsupported plist value types currently produce `Error::InvalidPlist`.

## IWA Archive Format

Each `.iwa` file is a Snappy-compressed stream of IWA chunks, each with a 4-byte header:

```
[kind:u8] [len_lo:u8] [len_mid:u8] [len_hi:u8] [compressed_payload: len bytes]
```

Only chunk kind `0` (Snappy) is currently supported.

After decompression, the archive byte stream begins with a length-prefixed header packet (IwaPacket), followed by the body. The header contains a protobuf-encoded `IwaArchiveDescriptor`:

- Field 1: root object ID (varint)
- Field 2 → Field 1: kind hint (varint, identifies the archive type)
- Field 2 → Field 3: body hint (body byte count)
- Field 2 → Field 4 (repeated): object references (id, kind_hint, state_hint)

The body is a stream of length-delimited protobuf messages. The body often starts with a run of "leading object reference" messages (field 1, wire type 2, containing inner field 1 = varint object ID). These are read by `IwaArchive::leading_object_references()`.

### Writing IWA archives

`IwaArchive::encode(header, body)` reverses the decode path: it length-prefixes the header packet, appends the body, and emits the result as Snappy chunks. Each chunk holds at most 64 KiB of decompressed bytes (the window real iWork writers use), and the payload is encoded as Snappy literal runs only (no back-references), which is valid Snappy that any reader can decompress. `IwaArchive::reencode()` round-trips a decoded archive losslessly: a reader observes the same header packet and body bytes (only the Snappy framing may differ). This re-encode reproduces every `.iwa` archive in the example documents.

`numbers::Workbook::to_numbers_bytes()` and
`numbers::Workbook::save_numbers()` synthesize a minimal direct-Index package
from scratch. The generated package includes:

- XML `Metadata/Properties.plist`
- minimal `Index/Document.iwa`, `Index/DocumentMetadata.iwa`,
  `Index/Metadata.iwa`, and `Index/DocumentStylesheet.iwa` archives
- compatibility-oriented `Index/ObjectContainer.iwa`,
  `Index/CalculationEngine.iwa`, `Index/ViewState.iwa`, and
  `Index/AnnotationAuthorStorage.iwa` archives
- generated `Index/Tables/DataList*.iwa` and `Index/Tables/Tile*.iwa`
  archives for scalar table cells

These packages are currently guaranteed only to round-trip through this crate's
reader. Opening them in Apple Numbers will require a fuller document object
graph with table references, stylesheet links, view state, and calculation
metadata.

### Reverse-engineering workflow

Use the graph tools when investigating the remaining object graph:

```bash
cargo run --example dump_iwa_graph -- path/to/document.numbers
cargo run --example diff_iwa_graph -- before.numbers after.numbers
```

`dump_iwa_graph` summarizes every `.iwa` archive in a package: descriptor
fields, object references, leading object references, chunk sizes, body length,
top-level body fields, recursive protobuf field shapes, and printable strings.
The strings are included only as landmarks for humans reading a dump; they are
not format evidence.

`diff_iwa_graph` compares two packages by entry set and by per-archive shape.
The intended workflow is to create controlled Apple Numbers fixtures that differ
by one operation, then diff them against each other and against this crate's
generated package. Stable structural deltas can be promoted into parser or
writer behavior; deltas that merely track authored content must stay out of the
format model.

## Stylesheet IWA Format (DocumentStylesheet.iwa)

After the leading object references, each message in the body is a style record with:

- Field 1: name (string)
- Field 2 or 5: object reference bytes → nested message field 1 = object ID
- Field 11: payload bytes → decoded as `StyleAttributes`

Style attribute payload fields:
- Field 1: bold flag (varint, 1=bold)
- Field 2: italic flag (varint, 1=italic)
- Field 3: font size (fixed32, IEEE 754 f32)
- Field 5: font name (bytes, UTF-8 string)

Records without field 11 may have attributes in nearby `0x5a` payload messages (field 11 of the outer stream). Enrichment searches the body for the sequence `0x08 <varint_object_id>` within 1000 bytes of a `0x5a` payload.

## DataList IWA Format (Index/Tables/DataList*.iwa)

DataList archives store typed lists of cell values. The archive body contains a single metadata message:

- Field 1: item count (varint)
- Field 2: item type (varint, 1=integer)

For **string DataLists**, additional body messages each represent one entry:
- Field 1: key (varint, 1-based)
- Field 2: count (varint, usually 1)
- Field 3: string value (bytes, UTF-8)

For **numeric DataLists**, the actual values are stored inline in the Tile row's field 6 (see below), not in the DataList archive. The DataList archive body only records the count.

## Tile IWA Format (Index/Tables/Tile*.iwa)

Tile archives store cell data in a stream of row messages in the body. Each row message:

- Field 1: row index (varint, 0-based; row 0 = header row)
- Field 2: column count (varint)
- Field 3: bytes[ncols × 12] — *legacy* (`_pre_bnc`) column metadata
- Field 4: bytes[255 × 2] — *legacy* (`_pre_bnc`) uint16 LE offsets (fixed 12-byte stride); superseded by field 7
- Field 5: varint (ncells count)
- Field 6: bytes[variable] — the current cell-storage buffer (see layout below)
- Field 7: bytes[255 × 2] — uint16 LE byte offsets into field 6 for each column's cell record; 0xffff = empty column. **This is the offset array to use** (not field 4).

### Field 6 Layout (wide-cell records)

Field 6 is the cell-storage buffer. Each column's record starts at its field-7 offset; records are **variable length** and each begins with the version byte `0x05`. `decode_cells` in `src/numbers/table.rs` implements this. The legacy field 3/4 `_pre_bnc` arrays use a fixed 12-byte stride and are ignored by the reader — reading field-4 offsets into field 6 lands mid-record for every cell after the first in real files. The writer still emits field 3/4 because real Numbers tiles retain them and consumers may expect those fields to exist.

Record header *(structurally grounded — verified across multiple real tile archives)*:

| Byte(s) | Meaning |
|---------|---------|
| 0       | version = `0x05` |
| 1       | cell type |
| 2-7     | reserved / sub-headers |
| 8-11    | flags bitmask (u32 LE) |
| 12+     | optional value fields, present in flag-bit order |

The low flag bits select the value field that follows at byte 12 (each consumes a fixed width, in bit order):

| Flag bit | Field | Width | Decoded as |
|----------|-------|-------|------------|
| `0x1`    | decimal128 number | 16 | `Number` |
| `0x2`    | IEEE 754 double   | 8  | `Number` |
| `0x4`    | date (seconds since Cocoa epoch) | 8 | `Date` |
| `0x8`    | string `DataList` key (u32 LE) | 4 | `Text` |

Higher bits (formula id, style ids, number-format id, …) follow but are not needed to recover the value, since the value fields are the lowest four bits and therefore appear first. The cell type byte (offset 1) is *not* used for value typing — the flags are authoritative.

**Decimal128.** Numbers stores numeric values as IEEE 754-2008 decimal128 (16 bytes, little-endian). The two high bytes hold the sign bit and biased exponent; bytes 0-13 plus the low bit of byte 14 form the coefficient. `decode_decimal128` converts to `f64`: `coefficient × 10^(exp)` where `exp = (((b[15] & 0x7f) << 7) | (b[14] >> 1)) - 0x1820` and the sign comes from `b[15] & 0x80`. This mirrors the well-known `numbers-parser` decode.

### String Lookup

String cells carry a u32 `DataList` key (flag bit `0x8`, at byte 12). Look it up in the string `DataList` archives (field 1 = key, field 3 = UTF-8 value); see `decode_string_datalist`.

### Date Encoding

Dates are f64 seconds since the Cocoa/NSDate epoch: **January 1, 2001, 00:00:00 UTC** (flag bit `0x4`). Example: 625,881,600 → November 3, 2020.

### Reader API Mapping

These format details surface through the public Numbers API like this:

- `numbers::Document::spreadsheet()` decodes the core document archives plus `Index/Tables/*.iwa`
- `Spreadsheet::table_archives()` exposes the raw decoded `DataList` and `Tile` archives
- `Spreadsheet::tables()` resolves string `DataList` entries first, then decodes row/cell values from each tile
- `TableRow::cells` contains `CellValue::{Empty, Text, Number, Date}`

The fixture-backed tests assert structural properties of decoded rows and cells
without using authored values from the examples as format scaffolding.

## Pages String Field Extraction

Pages documents in this repository are decoded as IWA/protobuf archives, but the
Pages object graph is not fully mapped yet. The current Pages reader therefore
takes a narrow structural approach:

- it decodes `Index/Document.iwa` as an `IwaArchive`
- it skips leading object-reference records
- it walks protobuf wire fields and nested length-delimited messages
- it returns length-delimited values only when they are valid UTF-8 text fields

This powers `pages::Document::document()`, which returns:

- `None` for title until the title object field is structurally decoded
- an empty heading list until heading/paragraph style fields are structurally decoded
- ordered UTF-8 string fields recovered from the document archive

This is intentionally not a semantic classifier. It does not scan raw printable
byte runs, filter strings by matching fixture-specific words, or infer headings
from authored text.

Current known limitations:

- returned strings may include non-prose metadata fields because the full schema
  is not decoded yet
- the parser does not yet reconstruct titles, headings, paragraph boundaries,
  text-run styling, or anchored object placement

## Keynote String Field Extraction

Keynote string extraction works slide-by-slide:

- it decodes each slide-related archive under `Index/`
- it skips leading object-reference records
- it walks protobuf wire fields and nested length-delimited messages
- it returns length-delimited values only when they are valid UTF-8 text fields

This powers `keynote::Document::presentation()`, which returns a list
of semantic slides containing:

- the source archive path
- whether the archive is a template slide
- `None` for layout name until that object field is structurally decoded
- `None` for title until that object field is structurally decoded
- ordered UTF-8 string fields
- an empty media description list until media/alt-text fields are structurally decoded

Current known limitations:

- slide ordering is based on archive-path sorting rather than a fully decoded
  presentation graph
- template and live slides are both surfaced when they contain UTF-8 fields
- presenter notes, animations, and exact on-slide object structure are not yet parsed
