use iwork::numbers::{CellValue, Workbook, WritableTable};

fn main() -> Result<(), iwork::Error> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/generated.numbers".to_owned());

    let mut workbook = Workbook::new();
    let mut table = WritableTable::new("Summary");
    table.push_row(vec![
        CellValue::Text("Category".to_owned()),
        CellValue::Text("Budget".to_owned()),
        CellValue::Text("Actual".to_owned()),
        CellValue::Text("Difference".to_owned()),
    ]);
    table.push_row(vec![
        CellValue::Text("Consulting".to_owned()),
        CellValue::Number(1000.0),
        CellValue::Number(850.0),
        CellValue::Number(150.0),
    ]);
    workbook.add_table(table);

    std::fs::write(&output, workbook.encode_scaffold_package()?)?;
    println!("{output}");
    Ok(())
}
