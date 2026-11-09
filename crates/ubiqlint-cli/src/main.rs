use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ubiqlint_core::{CONFIG_FILE_NAME, Config, DICTIONARY_FILE_NAME, Dictionary};

/// A linter for a ubiquitous language.
#[derive(Parser)]
#[command(name = "ubiqlint", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check that dictionaries (`terms.yaml`) and configuration files
    /// (`ubiqlint.yaml`) are well-formed
    Validate {
        /// Files to check. Each file's kind is decided by its name.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Validate { files } => validate(&files),
    }
}

/// Prints every problem in `files` and fails if there was any.
fn validate(files: &[PathBuf]) -> ExitCode {
    let mut valid = true;
    for path in files {
        if let Err(problems) = validate_file(path) {
            valid = false;
            for problem in problems {
                println!("{}", single_line(&problem));
            }
        }
    }
    if valid {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Escapes control characters other than tabs, so that each problem stays on
/// one line even when it quotes a value containing a line break.
fn single_line(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c != '\t' {
            line.extend(c.escape_default());
        } else {
            line.push(c);
        }
    }
    line
}

fn validate_file(path: &Path) -> Result<(), Vec<String>> {
    let shown = path.display();
    let read: fn(&str) -> Result<(), Vec<ubiqlint_core::Diagnostic>> =
        match path.file_name().and_then(|name| name.to_str()) {
            Some(DICTIONARY_FILE_NAME) => |source| Dictionary::from_yaml_str(source).map(drop),
            Some(CONFIG_FILE_NAME) => |source| Config::from_yaml_str(source).map(drop),
            _ => {
                return Err(vec![format!(
                    "{shown}: error: cannot tell what kind of file this is: name a dictionary \
                     `{DICTIONARY_FILE_NAME}` and a configuration file `{CONFIG_FILE_NAME}`"
                )]);
            }
        };
    let source = fs::read_to_string(path)
        .map_err(|error| vec![format!("{shown}: error: cannot read the file: {error}")])?;
    read(&source).map_err(|diagnostics| {
        diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "{shown}:{}: error: {}",
                    diagnostic.location, diagnostic.message
                )
            })
            .collect()
    })
}
