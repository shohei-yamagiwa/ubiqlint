use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ubiqlint_core::{
    CONFIG_FILE_NAME, Checker, Config, DICTIONARY_FILE_NAME, Diagnostic, Dictionary,
    LanguageAdapter,
};
use ubiqlint_java::JavaAdapter;

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
    /// Check the source files under a configuration file's directory
    /// against its rules
    Check {
        /// The configuration file to apply
        #[arg(long, default_value = CONFIG_FILE_NAME)]
        config: PathBuf,
        /// The dictionary to check against. Defaults to `terms.yaml` in the
        /// directory of the configuration file
        #[arg(long)]
        dictionary: Option<PathBuf>,
    },
}

/// The adapters for the languages ubiqlint can check.
const ADAPTERS: [&dyn LanguageAdapter; 1] = [&JavaAdapter];

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Validate { files } => validate(&files),
        Command::Check { config, dictionary } => check(&config, dictionary.as_deref()),
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

/// Checks every source file under the configuration file's directory,
/// printing every problem as `path:line:column: error: message`, and fails
/// if there was any.
fn check(config_path: &Path, dictionary: Option<&Path>) -> ExitCode {
    let dictionary_path = dictionary.map_or_else(
        || {
            config_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(DICTIONARY_FILE_NAME)
        },
        Path::to_owned,
    );
    let Some(config) = load(config_path, Config::from_yaml_str) else {
        return ExitCode::FAILURE;
    };
    let Some(dictionary) = load(&dictionary_path, Dictionary::from_yaml_str) else {
        return ExitCode::FAILURE;
    };

    let root = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(""));
    let read_root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    let mut files = Vec::new();
    collect_files(read_root, root, &mut files);
    files.sort_by(|(_, left), (_, right)| left.cmp(right));

    let checker = Checker::new(&dictionary, &config);
    let mut failed = false;
    for (read_path, display) in &files {
        let extension = read_path
            .extension()
            .and_then(|extension| extension.to_str());
        let Some(adapter) = extension.and_then(|extension| {
            ADAPTERS
                .iter()
                .find(|adapter| adapter.extensions().contains(&extension))
                .copied()
        }) else {
            continue;
        };
        let shown = display.display();
        let source = match fs::read_to_string(read_path) {
            Ok(source) => source,
            Err(error) => {
                failed = true;
                println!("{shown}: error: cannot read the file: {error}");
                continue;
            }
        };
        let relative = display.strip_prefix(root).unwrap_or(display);
        match adapter.extract(&source) {
            Err(errors) => {
                failed = true;
                for error in &errors {
                    println!(
                        "{shown}:{}: error: {}",
                        error.location,
                        single_line(&error.message)
                    );
                }
            }
            Ok(symbols) => {
                let diagnostics = checker.check(&symbols, relative);
                failed |= !diagnostics.is_empty();
                for diagnostic in &diagnostics {
                    println!(
                        "{shown}:{}: error: {}",
                        diagnostic.location,
                        single_line(&diagnostic.message)
                    );
                }
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Collects every regular file under `read_base`, paired with its path
/// relative to `rel_base`, for display and rule matching.
fn collect_files(read_base: &Path, rel_base: &Path, files: &mut Vec<(PathBuf, PathBuf)>) {
    let Ok(entries) = fs::read_dir(read_base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let read = read_base.join(&name);
        let relative = rel_base.join(&name);
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => collect_files(&read, &relative, files),
            Ok(file_type) if file_type.is_file() => files.push((read, relative)),
            _ => {}
        }
    }
}

/// Reads `path` and parses it, printing every problem found.
fn load<T>(path: &Path, parse: fn(&str) -> Result<T, Vec<Diagnostic>>) -> Option<T> {
    let shown = path.display();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            println!("{shown}: error: cannot read the file: {error}");
            return None;
        }
    };
    match parse(&source) {
        Ok(value) => Some(value),
        Err(diagnostics) => {
            for diagnostic in &diagnostics {
                println!(
                    "{shown}:{}: error: {}",
                    diagnostic.location,
                    single_line(&diagnostic.message)
                );
            }
            None
        }
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
