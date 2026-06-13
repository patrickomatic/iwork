use std::path::PathBuf;

#[path = "../support/iwa_graph.rs"]
mod iwa_graph;

fn main() -> Result<(), iwork::Error> {
    let mut args = std::env::args().skip(1);
    let left = PathBuf::from(
        args.next()
            .ok_or(iwork::Error::InvalidIwa("missing left package path"))?,
    );
    let right = PathBuf::from(
        args.next()
            .ok_or(iwork::Error::InvalidIwa("missing right package path"))?,
    );

    print!("{}", iwa_graph::diff_packages(&left, &right)?);
    Ok(())
}
