# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo run -- <path-to-iwork-file>   # CLI inspector
```

Before committing, always run `cargo test --workspace && cargo clippy --workspace -- -D warnings` and fix all errors.

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

## Tests

Integration tests live in `tests/harness.rs` and are fixture-driven against 9 real iWork example files in `examples/numbers/`, `examples/pages/`, and `examples/keynote/`. Tests verify that all examples open, expose metadata, and contain a stylesheet signal. Unit tests for internal modules are in `src/tests.rs`.

## Format Reference

`docs/file-format.md` documents the reverse-engineered ZIP, plist, IWA, and protobuf structures. Read this before modifying `package.rs`, `iwa.rs`, or `protobuf.rs`.

## Current Numbers Decoding State

The Numbers reader has an evidence-backed table path:

- `Spreadsheet::sheets()` decodes `Sheet` objects from `Index/Document.iwa`, with field 1 as the sheet name and field 2 filtered to `TableInfo` references, then resolved through `TableInfo -> TableModel`.
- `Spreadsheet::table_models()` decodes `TableModel` objects from `Index/CalculationEngine.iwa` (with `Document.iwa` fallback), including table UUID, name, row/column counts, header row/column counts, tile ids, header storage bucket ids, and DataList references.
- `Spreadsheet::decoded_tables()` is the authoritative table view: it follows each model's tiles and scoped DataLists, merges multi-tile row ranges, and avoids cross-table string-key collisions.
- `Spreadsheet::formula_records()` decodes type-4008 formula records from `Index/CalculationEngine.iwa`; field 1 is exposed as a raw `FormulaRecordKey`, field 2 matches formula ids stored on some formula-result cells, fields 7/8 are exposed as raw `FormulaBoundsPair` values, and referenced type-4009 auxiliary record ids are retained.
- `Spreadsheet::formula_auxiliary_records()` decodes type-4009 formula auxiliary records structurally: raw fields 1-3 plus repeated field-4 entries with raw fields 1-2 and optional nested two-varint field-6 payloads.
- `Spreadsheet::header_storage_bucket()` decodes type-6006 `HeaderStorageBucket` archives structurally. Each table model references a row-indexed bucket via `DataStore.field 1.2` and a column-indexed bucket via `DataStore.field 2`; entry fields 2-4 are still raw structural fields.
- Cell decoding currently covers `Empty`, plain text, rich text, numbers/decimal128, dates, booleans, durations, formula errors, cached formula results with formula ids (`CellValue::Formula`), currency, and percentages.

Known Numbers gaps:

- Formula expressions/dependency graph are not decoded; formula result cells preserve the formula-result marker, formula id, and cached value through `CellValue::Formula`, and some formula ids resolve to type-4008 `FormulaRecord` objects. FormulaRecord fields 7/8 and type-4009 auxiliary records are decoded structurally but their exact graph semantics are not named yet.
- Header storage bucket axis roles are decoded, but layout semantics for entry fields 2-4 are not fully cross-validated.
- Pivot table semantics are not modeled beyond normal sheet/table object membership and decoded cell values.
- Writer output is crate-readable but still not guaranteed to open in Apple Numbers because the full document/table object graph, view state, styles, and calculation metadata are incomplete.

## Reverse-Engineering Discipline

When investigating binary format details, distinguish sharply between:

- **Structural invariants** — byte patterns that hold regardless of document content (type bytes, sentinel values, fixed flags). These are safe to build against.
- **Data-inferred guesses** — interpretations that only work because the example file happened to contain certain values (e.g., "this u32 looks like a dollar amount"). These are not reliable format knowledge.

The example files in `examples/` contain arbitrary user data. Never use the actual cell values (prices, amounts, dates, strings) to reason about what an encoding means — those values are content, not format. A pattern that only appears to work because the example has budget data may break entirely on a different document.

### Reverse-Engineering Tooling

Do format investigation with the committed tools, **not throwaway scripts**. `protorev` (https://github.com/patrickomatic/protorev, consumed by the `iwork-workbench` package) is the protobuf reverse-engineering workbench; the workbench binaries decode the IWA/Snappy framing and feed raw object payloads to it. Don't hand-roll protobuf wire decoding or shape inference in examples — delegate to `protorev`.

- `cargo run -p iwork-workbench --bin dump_iwa_graph -- <file>` / `diff_iwa_graph -- <a> <b>` — IWA framing + object-stream overview; per-archive shape and diff come from a `protorev` `Corpus`.
- `cargo run -p iwork-workbench --bin inspect_numbers -- <file> [name-filter]` — per-object protobuf dump via `protorev::dump_message`.
- `cargo run -p iwork-workbench --bin iwa_corpus -- {schema|infer|explain|values|diff} <type> [<field.path>] <file>...` — runs `protorev`'s full feature set over every object of one message type (`<type>` is an iWork message-type id; `<field.path>` is a dotted path like `4.3`).
- `cargo run -p iwork-workbench --bin iwa_corpus -- experiments <type> <manifest.protorev>` — runs a multi-experiment before/after diff from a `.protorev` manifest file. The manifest lists iWork package paths (not raw `.pb` files); this is the right tool for controlled one-variable investigations (e.g. "same slide, different layout").
- `cargo run -p iwork-workbench --bin iwa_refs -- {types|edges|refs} ... <file>...` — cross-object reference graph (object-graph level; not something `protorev` covers).
- `cargo run -p iwork-workbench --bin dump_cells -- <file> [--limit N]` — wide-cell record dump (type byte / flags / payload) for table tiles, plus a type-byte→flag-mask summary. Interprets the opaque field-6 cell buffer, which `protorev` cannot see into; protobuf framing is still delegated to `ProtoMessage`.

Boundaries: `protorev` is a dependency of the `iwork-workbench` package; the library never depends on it. `src/protobuf.rs` remains the production decoder.

`protorev`'s confidence gating encodes the structural-vs-data-inferred rule above: a field observed in every relevant sample is `high` confidence; a sparsely observed one stays `medium`/`low` until corroborated across controlled fixtures. Promote a field into parser/writer behavior only once the evidence is `high` or cross-validated independently (e.g. tile-decoded geometry matching a declared count).

### Funneling RE Insights Back into protorev

When reverse engineering surfaces a generalizable protobuf behavior — not iWork-specific, but a pattern that affects how any schema should be read — file it as an issue or PR on the `protorev` repo rather than working around it locally. The boundary is: iWork-specific object-graph conventions (cross-archive ID references, IWA message-type routing) stay in `iwork`; generalizable wire-level or corpus-inference behaviors belong in `protorev`.

Known cases to upstream:

- **Control-delimited UTF-8 (TSWP type-2001 field 3)**: `protorev`'s `LengthDelimitedHints` rejects this field as non-UTF8 because it contains embedded non-whitespace control bytes (`\x04` as block delimiters). The field *is* valid UTF-8 text with embedded structural markers. A `control_delimited_utf8` hint — fires when a byte string is clean UTF-8 after stripping non-whitespace control chars — would surface this class of field automatically. Currently the only way to find it is hex inspection.

When implementing new iwork decoders: if the field was invisible to `protorev` and would have been visible with a better hint, flag it for upstreaming before closing the task.
