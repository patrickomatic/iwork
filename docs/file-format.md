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
- Field 3: bytes[ncols × 12] — column metadata (byte 0 of each 12-byte entry = 0x04 for all column types seen)
- Field 4: bytes[255 × 2] — uint16 LE byte offsets into field 6 for each column's cell record; 0xffff = empty column
- Field 5: varint (ncells count)
- Field 6: bytes[variable] — packed cell and value data (see layout below)
- Field 7: bytes[255 × 2] — uint16 LE byte offsets into field 6 for each column's style record

### Field 6 Layout

Field 6 is a flat byte buffer. Offsets in field 4 and field 7 index into it. Offsets are NOT necessarily 12-byte aligned — field 7 offsets can overlap with cell records.

**Primary cell records** (at field 4 offsets, 12 bytes each):

| Byte(s) | Type 0x05 (style) | Type 0x03 (string) | Type 0x00 byte2=0x00 (date) | Type 0x00 byte2=0x80 (number) |
|---------|-------------------|--------------------|-----------------------------|-------------------------------|
| 0       | 0x05              | 0x03               | 0x00                        | 0x00                          |
| 1       | extra flags       | extra flags        | 0x00                        | 0x00                          |
| 2       | ...               | 0x00               | 0x00                        | 0x80                          |
| 3       | ...               | 0x00               | 0x00                        | 0x00                          |
| 4-7     | style data        | DataList key (u32 LE) | date value high bytes   | format ref (u32 LE, constant) |
| 0-7     | —                 | —                  | f64 LE seconds since epoch  | —                             |
| 8-11    | style data        | rich text style    | format DataList key (u32)   | DataList key (u32 LE, varies) |

**Known-unknown cell type bytes**: `my_stocks.numbers` contains cell records with type bytes `0x01`, `0x02`, `0x04`, `0x0e`, `0x0f`, and `0xcb` that are not yet understood. These likely encode numeric values (prices, percentages, market caps) with different encodings than the `0x00/0x80` path above. Currently decoded as `CellValue::Empty`.

**Inline numeric value area** *(structurally grounded)*: starts at `max(field7_non_sentinel_offsets) + 12` — the byte immediately after the last field 7 style record. This formula is general and derives from the field 7 offset array, not from any hardcoded position.

**Inline numeric value encoding** *(provisional, observed in one file)*: the first 4 bytes at the inline area start are a u32 LE value for the first numeric column; subsequent numeric columns follow at +12 byte intervals. The unit (whole dollars, cents, etc.) depends on the format DataList referenced at bytes 8-11 of the cell record. This was observed in `personal_budget.numbers` only and may not generalise.

### Date Encoding

Dates are stored as f64 seconds since the Cocoa/NSDate epoch: **January 1, 2001, 00:00:00 UTC**.

Example: 625,881,600 seconds → November 3, 2020.

### String Lookup

String cell records (type 0x03) store a u32 DataList key at bytes 4-7. Look up the corresponding string in the matching DataList archive (field 3 = string value).

### Numeric Value Encoding

For type `0x00` / sub-type `0x80` cells: the value is stored in the inline area (see above), not in the cell record itself. Bytes 4-7 of the cell record are a constant format reference; bytes 8-11 are the format DataList key (varies per column, constant per row). The format DataList encodes display precision. The actual numeric unit (whole dollars, cents, etc.) is not yet determined structurally.

Other numeric cell types (`0x01`, `0x02`, `0x04`, etc. — seen in `my_stocks.numbers`) are not yet decoded. The value location and encoding for those types is unknown.

## Write Behavior

Current write support is package-preserving, not format-rewriting.

When you call `write`, the crate writes the original package bytes back out unchanged. That gives us a strong round-trip guarantee for the currently supported workflows, but it also means the crate is not yet performing semantic edits to package contents.
