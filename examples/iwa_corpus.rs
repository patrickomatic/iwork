//! Reverse-engineering bridge from iWork IWA archives to the `protorev` workbench.
//!
//! It decodes the IWA/Snappy framing with the `iwork` crate, then feeds the raw
//! protobuf payloads of one object type into `protorev` for schema inference,
//! field explanation, value sampling, and controlled before/after diffing. This
//! replaces the ad-hoc extraction scripts that hand-rolled the same work.
//!
//! ```text
//! cargo run --example iwa_corpus -- schema      6001 [--min-confidence high|medium|low] examples/numbers/*.numbers
//! cargo run --example iwa_corpus -- infer       6001 examples/numbers/*.numbers
//! cargo run --example iwa_corpus -- explain     6001 9   examples/numbers/*.numbers
//! cargo run --example iwa_corpus -- values      6001 8   examples/numbers/*.numbers
//! cargo run --example iwa_corpus -- diff        6001 before.numbers after.numbers
//! cargo run --example iwa_corpus -- experiments 6001 experiments.protorev
//! ```
//!
//! The object type is the Apple iWork message type identifier (see
//! `numbers::message_type_name`); the field path is a `protorev` dotted path such
//! as `4.3` (`TableModel` → `DataStore` → `TileStorage`).
//!
//! `experiments` reads a `.protorev` manifest (protorev's `[[experiment]]` format)
//! where the `before`/`after` entries are iWork package paths instead of raw `.pb`
//! files. For each experiment it collects objects of `<type>` from both sides,
//! builds corpora, and runs `Corpus::diff`.

use std::error::Error;
use std::path::Path;

use iwork::numbers::message_type_name;
use iwork::{IwaArchive, Package};
use protorev::{Confidence, Corpus, ExperimentManifest, FieldPath, Message, SchemaOptions};

const CORPUS_DEPTH: usize = 8;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first() else {
        print_usage();
        return Ok(());
    };
    let rest = &argv[1..];

    match command.as_str() {
        "schema" => schema_command(rest),
        "infer" => infer_command(rest),
        "explain" => explain_command(rest),
        "values" => values_command(rest),
        "diff" => diff_command(rest),
        "experiments" => experiments_command(rest),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(err(format!("unknown command {other:?}"))),
    }
}

fn schema_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut min_confidence = Confidence::High;
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--min-confidence" {
            let level = iter.next().ok_or_else(|| err("--min-confidence needs a value"))?;
            min_confidence =
                Confidence::parse(level).ok_or_else(|| err(format!("invalid confidence {level:?}")))?;
        } else {
            positional.push(arg.clone());
        }
    }

    let (message_type, files) = split_type_and_files(&positional)?;
    let corpus = corpus_for(message_type, files)?;
    print!("{}", corpus.schema(&SchemaOptions { min_confidence }));
    Ok(())
}

fn infer_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (message_type, files) = split_type_and_files(args)?;
    let corpus = corpus_for(message_type, files)?;
    print!("{}", corpus.summary());
    Ok(())
}

fn explain_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (message_type, path, files) = split_type_path_files(args)?;
    let corpus = corpus_for(message_type, files)?;
    let output = corpus
        .explain(&path)
        .ok_or_else(|| err(format!("field {path} was not observed in the corpus")))?;
    print!("{output}");
    Ok(())
}

fn values_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (message_type, path, files) = split_type_path_files(args)?;
    let messages = collect_messages(message_type, files)?;
    report_corpus(message_type, messages.len());
    let corpus = Corpus::from_messages(&messages, CORPUS_DEPTH);
    let output = corpus
        .values(&messages, &path)
        .ok_or_else(|| err(format!("field {path} had no observed values in the corpus")))?;
    print!("{output}");
    Ok(())
}

fn diff_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let message_type = parse_type(args.first())?;
    let before = args.get(1).ok_or_else(|| err("missing <before.numbers>"))?;
    let after = args.get(2).ok_or_else(|| err("missing <after.numbers>"))?;

    let before_corpus = corpus_for(message_type, std::slice::from_ref(before))?;
    let after_corpus = corpus_for(message_type, std::slice::from_ref(after))?;
    print!("{}", Corpus::diff(&before_corpus, &after_corpus));
    Ok(())
}

/// Runs all experiments in a `.protorev` manifest against iWork packages.
///
/// Each `[[experiment]]` block lists before/after iWork package paths. For each
/// experiment the example collects all objects of `message_type`, builds corpora,
/// and prints a `Corpus::diff`. This mirrors `protorev experiments` but with the
/// IWA/Snappy layer in front so you work directly with `.numbers`/`.pages`/`.key`
/// files rather than pre-extracted `.pb` payloads.
fn experiments_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let message_type = parse_type(args.first())?;
    let manifest_path = args.get(1).ok_or_else(|| err("missing <manifest.protorev>"))?;

    let manifest = ExperimentManifest::from_file(manifest_path)?;
    for experiment in &manifest.experiments {
        println!("=== {} ===", experiment.name);
        if let Some(notes) = &experiment.notes {
            println!("notes: {notes}");
        }

        let before_files: Vec<String> = experiment
            .before
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let after_files: Vec<String> = experiment
            .after
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        let before_corpus = corpus_for(message_type, &before_files)?;
        let after_corpus = corpus_for(message_type, &after_files)?;
        print!("{}", Corpus::diff(&before_corpus, &after_corpus));
        println!();
    }
    Ok(())
}

/// Builds a `protorev` corpus from every object of `message_type` in `files`.
fn corpus_for(message_type: u64, files: &[String]) -> Result<Corpus, Box<dyn Error>> {
    let messages = collect_messages(message_type, files)?;
    report_corpus(message_type, messages.len());
    Ok(Corpus::from_messages(&messages, CORPUS_DEPTH))
}

/// Decodes the IWA framing and returns the protobuf payload of every object that
/// declares `message_type`, across all `files`.
fn collect_messages(message_type: u64, files: &[String]) -> Result<Vec<Message>, Box<dyn Error>> {
    let mut messages = Vec::new();
    for file in files {
        let package = Package::open(file)?;
        for entry in package.entries() {
            if !Path::new(&entry.path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("iwa"))
            {
                continue;
            }
            let Ok(archive) = IwaArchive::decode(package.entry_bytes(&entry.path)?) else {
                continue;
            };
            for object in archive.objects() {
                if object.message_type == Some(message_type)
                    && let Ok(message) = Message::decode(&object.payload)
                {
                    messages.push(message);
                }
            }
        }
    }
    Ok(messages)
}

fn report_corpus(message_type: u64, count: usize) {
    let name = message_type_name(message_type).unwrap_or("?");
    eprintln!("type {message_type} ({name}): {count} object payload(s)");
}

fn split_type_and_files(positional: &[String]) -> Result<(u64, &[String]), Box<dyn Error>> {
    let (first, files) = positional
        .split_first()
        .ok_or_else(|| err("missing <type>"))?;
    if files.is_empty() {
        return Err(err("missing <file.numbers>..."));
    }
    Ok((parse_type(Some(first))?, files))
}

fn split_type_path_files(args: &[String]) -> Result<(u64, FieldPath, &[String]), Box<dyn Error>> {
    let message_type = parse_type(args.first())?;
    let path_raw = args.get(1).ok_or_else(|| err("missing <field.path>"))?;
    let path =
        FieldPath::parse(path_raw).ok_or_else(|| err(format!("invalid field path {path_raw:?}")))?;
    let files = &args[2..];
    if files.is_empty() {
        return Err(err("missing <file.numbers>..."));
    }
    Ok((message_type, path, files))
}

fn parse_type(arg: Option<&String>) -> Result<u64, Box<dyn Error>> {
    let raw = arg.ok_or_else(|| err("missing <type>"))?;
    raw.parse::<u64>()
        .map_err(|_| err(format!("invalid object type {raw:?}")))
}

fn err(message: impl Into<String>) -> Box<dyn Error> {
    Box::<dyn Error>::from(message.into())
}

fn print_usage() {
    println!("iwa_corpus: feed iWork object payloads into the protorev workbench");
    println!();
    println!("usage:");
    println!("  iwa_corpus schema      <type> [--min-confidence high|medium|low] <file>...");
    println!("  iwa_corpus infer       <type> <file>...");
    println!("  iwa_corpus explain     <type> <field.path> <file>...");
    println!("  iwa_corpus values      <type> <field.path> <file>...");
    println!("  iwa_corpus diff        <type> <before> <after>");
    println!("  iwa_corpus experiments <type> <manifest.protorev>");
    println!();
    println!("  <type> is an iWork message type id (e.g. 6001 for TableModel)");
    println!("  <file> is a .numbers, .pages, or .key package");
    println!("  experiments: before/after entries in the manifest are package paths");
}
