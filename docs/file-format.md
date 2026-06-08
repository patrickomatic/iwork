# iWork File Format Notes

This document records the file-format assumptions the crate currently depends on. It is not intended to be a complete description of Apple iWork internals; it describes the parts we have verified well enough to build against today.

## Container Format

All supported iWork documents in this repository are ZIP archives.

The crate currently relies on these ZIP-level properties:

- the file starts with a local file header signature `0x04034B50`
- the archive contains a standard end-of-central-directory record
- central directory records use the standard signature `0x02014B50`
- local file headers use the standard signature `0x04034B50`
- entry names are UTF-8
- entry data is only read from uncompressed entries with compression method `0`

The implementation does not currently inflate compressed ZIP members. If an entry needed by the reader is compressed, the crate returns `Error::UnsupportedCompression`.

## Package Layout

The package layout we currently rely on is small:

- `Metadata/Properties.plist`
- `Index/DocumentStylesheet.iwa`

`Metadata/Properties.plist` is used for stable metadata exposed by `PropertiesPlist`.

`Index/DocumentStylesheet.iwa` is currently treated as an opaque byte payload. The inspection path only scans it for simple keyword occurrences such as `bold`, `italic`, `underline`, `strikethrough`, and `font`.

The crate does not yet parse `.iwa` records structurally.

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

Field 6 is the cell-storage buffer. Each column's record starts at its field-7 offset; records are **variable length** and each begins with the version byte `0x05`. `decode_cells` in `src/numbers/table.rs` implements this. (The legacy field 3/4 `_pre_bnc` arrays use a fixed 12-byte stride and are ignored — reading field-4 offsets into field 6 lands mid-record for every cell after the first.)

Record header *(structurally grounded — verified across `Tile-1139365` and `Tile-1139370` in `my_stocks.numbers`)*:

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

**Decimal128.** Numbers stores numeric values as IEEE 754-2008 decimal128 (16 bytes, little-endian). The two high bytes hold the sign bit and biased exponent; bytes 0-13 plus the low bit of byte 14 form the coefficient. `decode_decimal128` converts to `f64`: `coefficient × 10^(exp)` where `exp = (((b[15] & 0x7f) << 7) | (b[14] >> 1)) - 0x1820` and the sign comes from `b[15] & 0x80`. Decimal storage is why values like `307.34` round-trip exactly. (This mirrors the well-known `numbers-parser` decode.)

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

The fixture-backed tests intentionally assert real header rows and representative values from
`personal_budget.numbers`, `pivot_table.numbers`, and `my_stocks.numbers` so changes to the
reverse-engineered row layout fail loudly.

## Pages Semantic Text Extraction

Pages documents in this repository do not currently expose their prose as clean,
typed protobuf string fields the way Numbers tables expose cells. The current
Pages semantic parser therefore takes a more conservative approach:

- it reads raw `Index/Document.iwa` bytes from the package
- it extracts printable string runs
- it filters out known locale, formatting, stylesheet, and UUID noise
- it normalizes high-confidence headings such as `Chapter N`

This powers `pages::Document::document()`, which returns:

- an optional title when a strong multi-word title candidate is present
- normalized headings (`Prologue`, `Subheading`, `Chapter 1`, ...)
- ordered text fragments recovered from the document archive

This is intentionally a **best-effort semantic layer**, not a complete model of
Pages paragraphs, text runs, or anchored objects. The fixture coverage asserts
recoverable content from `modern_novel.pages` and `term_paper.pages`.

Current known limitations:

- some visually contiguous titles are split across multiple archive fragments,
  so title recovery can legitimately return `None`
- extracted prose may still include partial template text or other nearby
  printable runs when content and formatting bytes are interleaved
- the parser does not yet reconstruct paragraph boundaries, text-run styling,
  or anchored object placement

## Keynote Semantic Slide Extraction

Keynote slide content in the current fixtures is often easier to recover from
individual `Slide*.iwa` and `TemplateSlide*.iwa` archives than from the top-level
document archive. The current semantic parser therefore works slide-by-slide:

- it decodes each slide-related archive under `Index/`
- it extracts printable strings from the archive body
- it filters out known locale, transition, UUID, and formatting noise
- it classifies layout names, placeholder titles, and media descriptions

This powers `keynote::Document::presentation()`, which returns a list
of semantic slides containing:

- the source archive path
- whether the archive is a template slide
- an optional layout name
- an optional title / placeholder title
- ordered text fragments
- media descriptions recovered from slide assets

Current known limitations:

- slide ordering is based on archive-path sorting rather than a fully decoded
  presentation graph
- template and live slides are both surfaced because both contain meaningful
  text in the current fixtures
- presenter notes, animations, and exact on-slide object structure are not yet parsed
- some recovered text still reflects placeholder/template phrasing rather than
  finalized authored content
