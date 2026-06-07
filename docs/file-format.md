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

## Write Behavior

Current write support is package-preserving, not format-rewriting.

When you call `write`, the crate writes the original package bytes back out unchanged. That gives us a strong round-trip guarantee for the currently supported workflows, but it also means the crate is not yet performing semantic edits to package contents.
