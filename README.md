# Apple iWork Rust Crate

`iwork` is a Rust crate for opening Apple iWork packages (`.numbers`, `.pages`, and `.key`), inspecting a small set of stable metadata, and extracting semantic content from supported document types.

## What This Crate Does

- opens iWork packages as ZIP containers
- exposes package entries and raw entry bytes
- can write stored ZIP packages and encode protobuf / IWA payloads
- reads `Metadata/Properties.plist`
- reads Numbers sheets, table models, and table cell values — text, rich text,
  numbers (decimal128), dates, booleans, durations, errors, cached formula
  results, currency, and percentages
- can build Numbers table archives from scratch for scalar cell data
- inspects `Index/DocumentStylesheet.iwa` for simple keyword signals
- extracts UTF-8 string fields from Pages documents and Keynote decks

## Current Guarantees

- `Document::open` accepts any supported iWork package
- `numbers::Document`, `pages::Document`, and `keynote::Document` enforce the expected file extension
- fixture coverage exists for Numbers, Pages, and Keynote examples in `examples/`

## Usage

Add the crate to your project, then open a document and inspect it:

```rust
use iwork::Document;

fn main() -> Result<(), iwork::Error> {
    let document = Document::open("examples/numbers/personal_budget.numbers")?;
    let report = document.inspect("personal_budget.numbers")?;

    println!("kind: {}", report.kind.as_str());
    println!(
        "file format version: {}",
        report.properties.file_format_version.as_deref().unwrap_or("<unknown>")
    );

    Ok(())
}
```

You can also work at the package level:

```rust
use iwork::Document;

fn main() -> Result<(), iwork::Error> {
    let document = Document::open("examples/pages/modern_novel.pages")?;
    let stylesheet = document
        .package()
        .entry_bytes("Index/DocumentStylesheet.iwa")?;

    println!("stylesheet bytes: {}", stylesheet.len());
    Ok(())
}
```

List a Numbers document's tables with their names, geometry, and cells:

```rust
use iwork::numbers;

fn main() -> Result<(), iwork::Error> {
    let document = numbers::Document::open("examples/numbers/my_stocks.numbers")?;

    for (model, table) in document.spreadsheet()?.decoded_tables() {
        println!(
            "{} — {}x{}, {} rows decoded",
            model.name().unwrap_or("(unnamed)"),
            model.row_count(),
            model.column_count(),
            table.rows().len(),
        );
    }

    Ok(())
}
```

List a Numbers document's sheets and the table models they contain:

```rust
use iwork::numbers;

fn main() -> Result<(), iwork::Error> {
    let spreadsheet = numbers::Document::open("examples/numbers/my_stocks.numbers")?
        .spreadsheet()?;

    for sheet in spreadsheet.sheets() {
        println!(
            "{}: {:?}",
            sheet.name().unwrap_or("(unnamed)"),
            sheet.table_model_ids()
        );
    }

    Ok(())
}
```

Read cell values from a Numbers spreadsheet:

```rust
use iwork::numbers::{self, CellValue};

fn main() -> Result<(), iwork::Error> {
    let document = numbers::Document::open("examples/numbers/my_stocks.numbers")?;

    for table in document.spreadsheet()?.tables() {
        for row in table.rows() {
            for cell in &row.cells {
                match cell {
                    CellValue::Text(s) => println!("text:   {s}"),
                    CellValue::Number(n) => println!("number: {n}"),
                    CellValue::Date(secs) => println!("date:   {secs} s since 2001-01-01"),
                    CellValue::Empty => {}
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
```

Extract UTF-8 string fields from a Pages document:

```rust
use iwork::pages;

fn main() -> Result<(), iwork::Error> {
    let document = pages::Document::open("examples/pages/term_paper.pages")?;
    let content = document.document()?;

    println!("title: {:?}", content.title());     // None until title fields are structurally decoded
    println!("headings: {:?}", content.headings()); // empty until heading fields are structurally decoded
    println!("first fragments: {:?}", &content.text_fragments()[..3.min(content.text_fragments().len())]);
    Ok(())
}
```

Extract UTF-8 string fields from a Keynote deck:

```rust
use iwork::keynote;

fn main() -> Result<(), iwork::Error> {
    let document = keynote::Document::open("examples/keynote/blueprint.key")?;
    let presentation = document.presentation()?;

    for slide in presentation.slides() {
        println!("{:?} {:?}", slide.layout_name(), slide.title());
        println!("text: {:?}", slide.text_fragments());
        println!("media: {:?}", slide.media_descriptions());
    }

    Ok(())
}
```

## Numbers Parsing Notes

The Numbers reader currently follows a two-stage model:

- `Spreadsheet::table_models()` decodes each table's name and grid geometry from
  its `TableModel` object (the authoritative table list)
- `Spreadsheet::sheets()` decodes sheet names and resolves sheet membership from
  `Sheet -> TableInfo -> TableModel`
- `Spreadsheet::decoded_tables()` follows each model's `DataStore` to its tiles
  and string / rich-text / format lists, returning one `(TableModel, Table)` per
  real table with cells resolved per-table (no cross-table string-key collisions)
- `TableModel::row_header_storage_bucket_id()`,
  `TableModel::column_header_storage_bucket_id()`, and
  `Spreadsheet::header_storage_bucket()` expose the row- and column-indexed
  `HeaderStorageBucket` objects referenced by each table model
- `Spreadsheet::table_archives()` exposes the raw `Index/Tables/*.iwa` archives
- `Spreadsheet::tables()` resolves those archives into decoded rows and [`CellValue`](src/numbers/table.rs) values (lower-level; one entry per tile)

`.iwa` archives are streams of typed objects; `IwaArchive::objects()` decodes the
full stream and `numbers::message_type_name()` names the known archive and table
object types (`Document`, `Sheet`, `TableInfo`, `TableModel`, `Tile`, `DataList`, …).

The write-side Numbers support can create minimal crate-readable `.numbers`
packages:

- `numbers::Workbook` and `numbers::WritableTable` let you assemble table rows from scratch
- `Workbook::encode_table_archives()` emits fresh `Tile` and string `DataList` archives for scalar cells
- `Workbook::to_numbers_bytes()` and `Workbook::save_numbers()` wrap those archives in a ZIP package with metadata, core IWA members, table archives, and compatibility-oriented stubs for object container, calculation engine, and view state

The generated package is currently intended for round-tripping through this
crate. Full Apple Numbers compatibility still requires decoding and generating
the real document/table object graph and its stylesheet, view-state, and
calculation-engine references.

Example tools help map those missing structures without turning fixture content
into parser heuristics. They decode the IWA/Snappy framing and hand the raw
protobuf payloads to the [`protorev`](https://github.com/patrickomatic/protorev) workbench rather than
re-implementing wire decoding:

```bash
# IWA framing + object stream overview, and a structural diff between packages
cargo run --example dump_iwa_graph -- path/to/document.numbers
cargo run --example diff_iwa_graph -- left.numbers right.numbers
cargo run --example inspect_numbers -- path/to/document.numbers

# Schema-infer one object type across packages (protorev schema/explain/values/diff/experiments)
cargo run --example iwa_corpus -- schema       6001 examples/numbers/*.numbers
cargo run --example iwa_corpus -- diff         6001 before.numbers after.numbers
cargo run --example iwa_corpus -- experiments  7    layout_investigation.protorev

# Explore the cross-object reference graph
cargo run --example iwa_refs -- edges 6001 examples/numbers/personal_budget.numbers
```

`dump_iwa_graph`/`diff_iwa_graph` work at the IWA framing and object-graph
level (entries, descriptor refs, object-type counts) with a `protorev` corpus
shape per archive. `iwa_corpus` gathers every object of one message type and
runs `protorev`'s confidence-gated schema inference, per-field explanations,
value sampling, and corpus diffing on it. `iwa_refs` recovers the cross-object
reference graph structurally (which object types reference which). Together they
let reverse engineering focus on controlled one-edit deltas, such as adding one
table, renaming one sheet, or changing one cell style.

The current parser relies on these reverse-engineered format details:

- string values are looked up through `DataList*.iwa` archives
- numeric and date values are stored inline in each tile row's field-6 cell buffer
- field 7 is the authoritative uint16 offset table for locating cell records
- date values are `f64` seconds since the Cocoa epoch (`2001-01-01T00:00:00Z`)
- decimal values may be stored as IEEE 754-2008 decimal128 and are converted to `f64`

The test suite covers both low-level decoder branches and fixture-backed structural checks for:

- text cells, multi-text rows, and grouped text rows
- finite numeric values decoded through the current cell-storage layout
- Cocoa-epoch date cells from the Numbers fixtures
- row decoding behavior around column counts, sentinels, and truncated records

## Pages Parsing Notes

The Pages reader decodes `Index/Document.iwa` as IWA/protobuf data. Object types
are structurally identified by their message-type ID; the parser reads only
fields with confirmed paths rather than scanning raw byte runs.

Current decoded structure:

- `Body::template_name()` — type-10001 field `1.3` (UTF-8 template identifier)
- `Body::text_fragments()` — type-2001 field 3 (TSWP text storage; `\n` = paragraph break, non-whitespace control bytes = block boundaries; U+FFFC object-replacement chars are filtered)
- `Body::media_descriptions()` — type-3005 field `1.8` (image alt-text)

Known gaps today:

- `Body::title()` returns `None` — paragraph style → "Title" classification not yet decoded
- `Body::headings()` returns empty — heading/body style classification not yet decoded
- page/section structure and inline tables are not yet modeled
- this is read-only semantic extraction, not a structured Pages editing model

## Keynote Parsing Notes

The Keynote reader works at the slide-archive level. Each `Slide*.iwa` and
`TemplateSlide*.iwa` entry is decoded as an IWA object stream; slides are
sorted by archive path.

Current decoded structure:

- `Slide::is_template()` — path prefix (`Index/TemplateSlide-` vs `Index/Slide-`)
- `Slide::title()` — type-7 drawable with field 2=2 (title placeholder) → field `1.4.1` (type-2001 object ID) → field 3 (UTF-8)
- `Slide::text_fragments()` — type-2001 field 3, same encoding as Pages; U+FFFC filtered
- `Slide::media_descriptions()` — type-3005 field `1.8` (image alt-text)
- `Presentation::theme_name()` — type-10 field `1.3` (UTF-8)

Known gaps today:

- `Slide::layout_name()` returns `None` — field path for layout name not yet located
- `Slide::speaker_notes()` not yet implemented — type-7 field 2=1 → `1.4.1` → type-2001 field 3 (pattern confirmed, ready to implement)
- slide ordering is by archive path, not a fully decoded slide-graph traversal
- animations, transitions, and exact object placement are not yet modeled

## Format Notes

The codebase depends on a small set of reverse-engineered format assumptions. Those notes live in [docs/file-format.md](docs/file-format.md), with the most important details also documented directly in `src/package.rs` and `src/plist.rs`.

## Repository Layout

- `src/`: library implementation and the small inspection CLI
- `examples/`: sample `.numbers`, `.pages`, and `.key` fixtures
- `tests/`: fixture-driven integration coverage
- `docs/`: format notes and implementation-oriented documentation

## License

MIT. See [LICENSE](LICENSE).
