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

**Inline numeric value area** *(structurally grounded)*: starts at `max(field7_non_sentinel_offsets) + 12` — the byte immediately after the last field 7 style record. This formula is general and derives from the field 7 offset array, not from any hardcoded position.

**Inline numeric value encoding** *(provisional, observed in one file)*: the first 4 bytes at the inline area start are a u32 LE value for the first numeric column; subsequent numeric columns follow at +12 byte intervals. The unit (whole dollars, cents, etc.) depends on the format DataList referenced at bytes 8-11 of the cell record. This was observed in `personal_budget.numbers` only and may not generalise.

### Date Encoding

Dates are stored as f64 seconds since the Cocoa/NSDate epoch: **January 1, 2001, 00:00:00 UTC**.

Example: 625,881,600 seconds → November 3, 2020.

### String Lookup

String cell records (type 0x03) store a u32 DataList key at bytes 4-7. Look up the corresponding string in the matching DataList archive (field 3 = string value).

### Numeric Value Encoding

For type `0x00` / sub-type `0x80` cells: the value is stored in the inline area (see above), not in the cell record itself. Bytes 4-7 of the cell record are a constant format reference; bytes 8-11 are the format DataList key (varies per column, constant per row). The format DataList encodes display precision. The actual numeric unit (whole dollars, cents, etc.) is not yet determined structurally.

### Formula-Result Cells *(resolved)*

Some numeric cells do not store their value inline — they reference it by a **u32 LE key in bytes 8-11** of the cell record, looked up in a separate *formula* `DataList`. This covers the records previously catalogued as Pattern A and Pattern D below.

**Formula DataList structure** (`DataList` archive with field 1 / listType = `3`):

- The list message's field 2 is the maximum key (`max_key`) in the list.
- Each entry is a field-3 sub-message: field 1 = the u32 key; field 5 = a nested value message. The f64 result is reached via field 5 → field 1 → field 4 (a `fixed64`, read with `f64::from_bits`). See `decode_formula_datalist` in `src/numbers/spreadsheet.rs`.

**Matching a Tile to its formula DataList.** A document contains several formula DataLists with disjoint key ranges. To pick the right one for a Tile, `scan_max_formula_key` finds the largest bytes-8-11 value that *varies across rows within a single column* (a constant value is a format reference, not a formula key), and the matcher selects the smallest DataList whose `max_key` covers it.

> **Pitfall (fixed):** non-formula columns also place varying values in bytes 8-11 (style/format indices that can read as large integers, e.g. `2565` in `my_stocks.numbers`). Left unbounded, the scan returns one of those, no DataList's `max_key` covers it, and *every* formula cell falls back to `Empty`. The scan is therefore capped by an `upper_bound` = the largest `max_key` across all formula DataLists (`formula_lists` sorted ascending → `.last()`); candidates above it are rejected. In `my_stocks.numbers` the formula list is `DataList-1139377` (`max_key=19`, 12 entries), so the bound is 19.

### Unknown Cell Encodings (Investigation Notes)

The following observations come from structural analysis of `my_stocks.numbers` (`Tile-1139365.iwa`). **Byte 0 of the cell record is not always a fixed type tag** — it varies per row for the same column in several observed patterns. Patterns A and D below are now decoded as formula-result cells (see above); Patterns B, C, and E remain `CellValue::Empty`.

**Pattern A — varying byte 0, bytes 1-7 constant per column, bytes 8-11 vary:**

```
col 1, row 1: [0f 00 00 00  04 00 00 00  01 00 00 00]
col 1, row 2: [10 00 00 00  04 00 00 00  12 00 00 00]
col 1, row 3: [11 00 00 00  04 00 00 00  0e 00 00 00]
```

- Bytes 0 and 8-11 both vary per row; bytes 1-7 are constant per column.
- Bytes 4-7 = `04 00 00 00` look like a format reference (same role as in 0x00/0x80 cells).
- The value encoding is not yet determined. Byte 0 as a u32 LE (together with bytes 1-3 = 0x00) gives small integers (15, 16, 17) that do not obviously encode the cell value.

**Pattern B — varying byte 0, wide variable bytes 5-11:**

```
col 6, row 1: [01 52 00 00 00  78 86 2b 86 17 01 00]
col 6, row 2: [01 52 00 00 00  a0 18 47 2a d0 00 00]
col 6, row 3: [01 52 00 00 00  10 53 9c e6 86 01 00]
```

- Bytes 0-4 = `01 52 00 00 00` are constant per column.
- Bytes 5-11 vary per row. Likely encode the cell value, but the encoding is unknown (not a standard f64 or u32).

**Pattern C — entirely constant record across all rows:**

```
col 5, all rows: [01 00 00 00 05 0a 00 00 00 00 00 08]
col 8, all rows: [02 00 00 00 02 00 00 00 05 0a 00 00]
col 11, all rows: [04 00 00 00 02 00 00 00 02 00 00 00]
```

- All 12 bytes are identical across every data row.
- Likely formula cells, cells computed from other cells, or some other indirection — no per-row value is stored directly in the record.

**Pattern D — type 0x00 with unexpected byte layout:**

```
col 3, row 1: [00 00 00 00  48 12 02 00  03 00 00 00]
col 3, row 2: [00 00 00 00  48 12 02 00  12 00 00 00]
col 3, row 3: [00 00 00 00  48 12 02 00  13 00 00 00]
```

- Byte 0 = 0x00, byte 2 = 0x00 → current code interprets as a date (f64 at bytes 0-7).
- But bytes 0-7 are `00 00 00 00 48 12 02 00` (u64 LE = 0x0002124800000000, a denormal f64 ≈ 0), which the decoder treats as `Empty`.
- Bytes 4-7 = `48 12 02 00` are constant per column (looks like a format reference).
- Bytes 8-11 vary per row (3, 18, 19) — same role as a DataList key.
- **Hypothesis**: this may be a numeric cell, not a date — the format-ref / DataList-key pattern matches the 0x00/0x80 number layout, suggesting byte 2 = 0x00 does not reliably distinguish dates from numbers.

**Pattern E — large varint-like byte 0 (high-byte set):**

```
col 10, row 1: [cb 61 01 00 00 00 00 00 00 00 24 b0]
col 10, row 2: [9d 9a 00 00 00 00 00 00 00 00 24 30]
col 10, row 3: [13 46 02 00 00 00 00 00 00 00 22 b0]
```

- Bytes 0-3 vary; bytes 4-9 = `00 00 00 00 00 00` are constant; bytes 10-11 vary per row.
- The `cb`, `9d` bytes have bit 7 set (varint continuation in LEB128). If bytes 0-N are a LEB128 varint and bytes 10-11 are a secondary field, the varint values are 12491, 3357, and 19 — all small integers, but their units are unknown.
- **Alternative**: bytes 0-3 are a u32 LE value (90571, 39581, 74259) with bytes 10-11 as a secondary key.

**Next steps**: Patterns A and D are resolved (formula-result cells, see above). For the remaining patterns B, C, and E, examine the DataList archives referenced by bytes 4-7 and 8-11 of these records. `DataList-1139369` (listType field1=9, 1616 bytes — the largest non-string, non-formula archive) is the prime suspect for carrying the wide values seen in Pattern B; its encoding is not yet decoded.

## Write Behavior

Current write support is package-preserving, not format-rewriting.

When you call `write`, the crate writes the original package bytes back out unchanged. That gives us a strong round-trip guarantee for the currently supported workflows, but it also means the crate is not yet performing semantic edits to package contents.
