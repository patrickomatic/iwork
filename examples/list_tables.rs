use iwork::numbers::{self, CellValue};

fn main() -> Result<(), iwork::Error> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/numbers/personal_budget.numbers".to_owned());
    let document = numbers::Document::open(&path)?;
    let spreadsheet = document.spreadsheet()?;

    for (model, table) in spreadsheet.decoded_tables() {
        println!(
            "{} — {}x{} ({} rows decoded)",
            model.name().unwrap_or("(unnamed)"),
            model.row_count(),
            model.column_count(),
            table.rows().len(),
        );
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
