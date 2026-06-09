use std::path::PathBuf;

#[path = "support/iwa_graph.rs"]
mod iwa_graph;

fn main() -> Result<(), iwork::Error> {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or(iwork::Error::InvalidIwa("missing package path"))?,
    );
    print!("{}", iwa_graph::dump_package(&path)?);
    Ok(())
}
