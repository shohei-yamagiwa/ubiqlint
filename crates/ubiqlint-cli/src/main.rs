use clap::Parser;

/// A linter for a ubiquitous language.
#[derive(Parser)]
#[command(name = "ubiqlint", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
