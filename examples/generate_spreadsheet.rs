use iwork::numbers::{CellValue, Workbook, WritableTable};

fn main() -> Result<(), iwork::Error> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/iwork-generated.numbers".to_owned());

    let mut workbook = Workbook::new();
    let mut table = WritableTable::new("Budget");
    table.push_row(vec![
        CellValue::Text("Category".to_owned()),
        CellValue::Text("Amount".to_owned()),
        CellValue::Text("When".to_owned()),
    ]);
    table.push_row(vec![
        CellValue::Text("Utilities".to_owned()),
        CellValue::Number(42.5),
        CellValue::Date(625_881_600.0),
    ]);
    workbook.add_table(table);

    workbook.save_numbers(&output)?;
    println!("{output}");
    Ok(())
}
