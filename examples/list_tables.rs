use iwork::numbers::{self, CellValue, Spreadsheet};

fn main() -> Result<(), iwork::Error> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/numbers/personal_budget.numbers".to_owned());
    let document = numbers::Document::open(&path)?;
    let spreadsheet = document.spreadsheet()?;

    for (model, table) in spreadsheet.decoded_tables() {
        println!(
            "{} — {}x{} headers {}x{} ({} rows decoded)",
            model.name().unwrap_or("(unnamed)"),
            model.row_count(),
            model.column_count(),
            model.header_row_count(),
            model.header_column_count(),
            table.rows().len(),
        );
        if let Some(id) = model.row_header_storage_bucket_id() {
            println!("  row header bucket: {}", bucket_summary(&spreadsheet, id));
        }
        if let Some(id) = model.column_header_storage_bucket_id() {
            println!(
                "  column header bucket: {}",
                bucket_summary(&spreadsheet, id)
            );
        }
        for row in table.rows().iter().take(3) {
            let cells = row
                .cells
                .iter()
                .map(display_cell)
                .collect::<Vec<_>>();
            println!("  row {}: {:?}", row.index, cells);
        }
    }

    Ok(())
}

fn bucket_summary(spreadsheet: &Spreadsheet, id: u64) -> String {
    spreadsheet.header_storage_bucket(id).map_or_else(
        || format!("{id} missing"),
        |bucket| {
            let entry_count = bucket.entries().len();
            let first = bucket.entries().first().map(|entry| entry.index());
            let last = bucket.entries().last().map(|entry| entry.index());
            let common_span = bucket
                .entries()
                .first()
                .map(|entry| entry.field4())
                .unwrap_or(0);
            format!("{id} entries={entry_count} first={first:?} last={last:?} span={common_span}")
        },
    )
}

fn display_cell(cell: &CellValue) -> String {
    match cell {
        CellValue::Empty => "<empty>".to_owned(),
        CellValue::Bool(value) => format!("bool:{value}"),
        CellValue::Duration(value) => format!("dur:{value}s"),
        CellValue::Error => "<error>".to_owned(),
        CellValue::Formula(value) => format!("formula:{}", display_cell(value)),
        CellValue::Text(value) => value.clone(),
        CellValue::Number(value) => format!("{value}"),
        CellValue::Percentage(value) => format!("{:.1}%", value * 100.0),
        CellValue::Currency { value, code } => {
            let sym = code.as_deref().unwrap_or("?");
            format!("{sym} {value}")
        }
        CellValue::Date(value) => format!("date:{value}"),
    }
}
