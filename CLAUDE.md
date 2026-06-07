# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build
cargo test
cargo clippy
cargo run -- <path-to-iwork-file>   # CLI inspector
```

To run a single test:
```bash
cargo test <test_name>
cargo test --test harness            # integration tests only
```

## Lint Configuration

Clippy is configured in `Cargo.toml` (not a separate file). Key enforced rules:
- `pedantic` warnings enabled
- `unwrap_used`, `expect_used`, `panic` are **denied** — always use `?` or explicit `match`
- `missing_errors_doc` and `must_use_candidate` are allowed

## Architecture

iwork is a Rust library (+ thin CLI) for reading Apple iWork packages (`.numbers`, `.pages`, `.key`). These files are ZIP archives containing a `Metadata/Properties.plist` and binary `Index/*.iwa` files.

### Three-layer abstraction

1. **`Package`** (`src/package.rs`) — ZIP-level reader. Enumerates entries and extracts raw bytes. Only uncompressed ZIP entries (method 0) are supported.
2. **`Document`** (`src/lib.rs`) — Wraps `Package`. Provides inspection, plist metadata, and stylesheet access. Extension-agnostic.
3. **App-specific types** (`src/numbers/`, `src/pages/`, `src/keynote/`) — Thin wrappers that validate the file extension and delegate to `Document`/`Package`.

### IWA parsing stack

`IwaArchive` (`src/iwa.rs`) decodes the binary IWA format: Snappy-compressed chunks → `IwaPacket` → `IwaObjectReference`. `ProtoMessage` (`src/protobuf.rs`) is a minimal hand-rolled protobuf decoder (varint, fixed32/64, length-delimited) — no generated code, no external schema.

### Other modules

- `src/plist.rs` — Narrow XML + binary plist parser; only supports types seen in `Metadata/Properties.plist`.
- `src/stylesheet.rs` — Heuristic keyword scan over `Index/DocumentStylesheet.iwa` to extract style names, fonts, and identifiers.
- `src/inspect.rs` — `InspectionReport` and keyword counting utilities.
- `src/kind.rs` — `DocumentKind` enum inferred from file extension.
- `src/error.rs` — Unified `Error` type covering IO, UTF-8, plist, IWA, and truncation errors.
- `src/main.rs` — CLI: opens a file, prints inspection report.

### Write behavior

Writes are lossless but non-semantic: the original package bytes are written back unchanged. No in-place editing of package contents is supported yet.

## Tests

Integration tests live in `tests/harness.rs` and are fixture-driven against 9 real iWork example files in `examples/numbers/`, `examples/pages/`, and `examples/keynote/`. Tests verify that all examples open, expose metadata, and contain a stylesheet signal. Unit tests for internal modules are in `src/tests.rs`.

## Format Reference

`docs/file-format.md` documents the reverse-engineered ZIP, plist, IWA, and protobuf structures. Read this before modifying `package.rs`, `iwa.rs`, or `protobuf.rs`.

## Reverse-Engineering Discipline

When investigating binary format details, distinguish sharply between:

- **Structural invariants** — byte patterns that hold regardless of document content (type bytes, sentinel values, fixed flags). These are safe to build against.
- **Data-inferred guesses** — interpretations that only work because the example file happened to contain certain values (e.g., "this u32 looks like a dollar amount"). These are not reliable format knowledge.

The example files in `examples/` contain arbitrary user data. Never use the actual cell values (prices, amounts, dates, strings) to reason about what an encoding means — those values are content, not format. A pattern that only appears to work because the example has budget data may break entirely on a different document.
