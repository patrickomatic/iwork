use iwork::Document;

fn main() {
    let mut args = std::env::args();
    let executable = args.next().unwrap_or_else(|| "iwork".to_owned());
    let Some(path) = args.next() else {
        eprintln!("usage: {executable} <file.numbers|file.pages|file.key>");
        std::process::exit(2);
    };

    match run(&path) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
}

fn run(path: &str) -> Result<(), iwork::Error> {
    let document = Document::open(path)?;
    let report = document.inspect(path.to_owned())?;

    println!("file: {}", report.path);
    println!("kind: {}", report.kind.as_str());
    println!("entries: {}", report.entry_count);
    println!("iwa entries: {}", report.iwa_count);
    println!(
        "file format version: {}",
        report
            .properties
            .file_format_version
            .as_deref()
            .unwrap_or("<unknown>")
    );
    println!(
        "document uuid: {}",
        report
            .properties
            .document_uuid
            .as_deref()
            .unwrap_or("<unknown>")
    );
    println!(
        "style keyword hits: bold={}, italic={}, underline={}, strikethrough={}, font={}",
        report.style_keyword_hits["bold"],
        report.style_keyword_hits["italic"],
        report.style_keyword_hits["underline"],
        report.style_keyword_hits["strikethrough"],
        report.style_keyword_hits["font"],
    );

    Ok(())
}
