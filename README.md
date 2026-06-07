# Apple Numbers Rust Crate

Rust crate for reading and writing files compatible with Apple Numbers.

## Status

This repository is in the early reverse-engineering stage. The first job is to build a reliable fixture-driven test harness around real `.numbers` documents, then grow support in narrow slices.

Current focus:

- open and inspect real Numbers packages from `examples/`
- identify stable package metadata and archive structure
- reverse engineer text and style encoding
- start with basic formatting such as bold, italic, underline, and strikethrough
- defer broad write support until read-path coverage is solid

## Repository layout

- `examples/`: sample `.numbers` files used for inspection and regression testing
- `tests/`: integration tests and future fixture-based coverage
- `src/`: crate implementation and command-line tooling

## Development plan

1. Package/container access.
2. IWA payload discovery and decoding.
3. Document metadata extraction.
4. Text runs and basic style attributes.
5. Small, well-tested write operations.

The first write milestone should be intentionally narrow: a minimal document edit that preserves package integrity and unrelated structures.

## License

MIT. See [LICENSE](LICENSE).
