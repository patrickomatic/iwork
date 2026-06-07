# Apple iWork Rust Crate

Rust crate for reading and inspecting Apple iWork packages, with current fixture coverage for Numbers, Pages, and Keynote documents.

## Status

This repository is in the early reverse-engineering stage. The first job is to build a reliable fixture-driven test harness around real iWork documents, then grow support in narrow slices.

Current focus:

- open and inspect real Numbers, Pages, and Keynote packages from `examples/`
- identify stable package metadata and archive structure
- reverse engineer text and style encoding
- start with basic formatting such as bold, italic, underline, and strikethrough
- defer broad write support until read-path coverage is solid

## Repository layout

- `examples/`: sample `.numbers`, `.pages`, and `.key` files used for inspection and regression testing
- `tests/`: integration tests and future fixture-based coverage
- `src/`: crate implementation and command-line tooling

## Development plan

1. Package/container access.
2. IWA payload discovery and decoding.
3. Document metadata extraction.
4. Text runs and basic style attributes.
5. Small, well-tested write operations.

The first write milestone should be intentionally narrow: a minimal document edit that preserves package integrity and unrelated structures.

## Current capabilities

- generic `Document` opening for supported iWork package types
- inspection reports that classify the input as Numbers, Pages, or Keynote by extension
- app-specific `pages::Document` and `keynote::Document` entry points that reject the wrong extension
- fixture coverage across all three document types

## License

MIT. See [LICENSE](LICENSE).
