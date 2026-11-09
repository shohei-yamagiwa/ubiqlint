use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ignore::WalkBuilder;
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
    let mut reporter = Reporter::new();
    match Cli::parse().command {
        Command::Validate { files } => validate(&mut reporter, &files),
        Command::Check { config, dictionary } => {
            check(&mut reporter, &config, dictionary.as_deref())
        }
    }
    reporter.finish()
}

/// Prints problems to stdout, one per line. Rust ignores SIGPIPE, so a
/// closed pipe (`ubiqlint check ... | head -1`) is a write error here, not
/// a panic: the run just fails with exit code 1.
struct Reporter {
    out: io::BufWriter<io::StdoutLock<'static>>,
    failed: bool,
    broken: bool,
}

impl Reporter {
    fn new() -> Self {
        Self {
            out: io::BufWriter::new(io::stdout().lock()),
            failed: false,
            broken: false,
        }
    }

    /// Prints one line, unless the pipe is already broken.
    fn line(&mut self, text: &str) {
        if self.broken {
            return;
        }
        if writeln!(self.out, "{text}").is_err() {
            self.broken = true;
            self.failed = true;
        }
    }

    /// Prints one problem and fails the run.
    fn problem(&mut self, text: &str) {
        self.failed = true;
        self.line(text);
    }

    fn finish(mut self) -> ExitCode {
        let _ = self.out.flush();
        if self.failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }
}

/// Checks that each file is a well-formed dictionary or configuration file.
fn validate(reporter: &mut Reporter, files: &[PathBuf]) {
    for path in files {
        let parse: fn(&str) -> Result<(), Vec<Diagnostic>> =
            match path.file_name().and_then(|name| name.to_str()) {
                Some(DICTIONARY_FILE_NAME) => |source| Dictionary::from_yaml_str(source).map(drop),
                Some(CONFIG_FILE_NAME) => |source| Config::from_yaml_str(source).map(drop),
                _ => {
                    reporter.problem(&format!(
                        "{}: error: cannot tell what kind of file this is: name a dictionary \
                         `{DICTIONARY_FILE_NAME}` and a configuration file `{CONFIG_FILE_NAME}`",
                        path.display()
                    ));
                    continue;
                }
            };
        load(reporter, path, parse);
    }
}

/// Checks every source file under the configuration file's directory,
/// printing every problem as `path:line:column: error: message`.
fn check(reporter: &mut Reporter, config_path: &Path, dictionary: Option<&Path>) {
    // Every checked path is relative to the directory of the configuration
    // file, which is also where the default dictionary lives.
    let root = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(""));
    let read_root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    let dictionary_path =
        dictionary.map_or_else(|| root.join(DICTIONARY_FILE_NAME), Path::to_owned);

    // Load both files before failing, so that every problem is reported in
    // one run.
    let config = load(reporter, config_path, Config::from_yaml_str);
    let dictionary = load(reporter, &dictionary_path, Dictionary::from_yaml_str);
    let (Some(config), Some(dictionary)) = (config, dictionary) else {
        return;
    };

    let checker = Checker::new(&dictionary, &config);
    for relative in collect_files(reporter, read_root) {
        if config.excludes(&relative) {
            continue;
        }
        let extension = relative
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
        let shown = root.join(&relative).display().to_string();
        let source = match fs::read_to_string(read_root.join(&relative)) {
            Ok(source) => source,
            Err(error) => {
                reporter.problem(&format!("{shown}: error: cannot read the file: {error}"));
                continue;
            }
        };
        match adapter.extract(&source) {
            Err(errors) => {
                for error in &errors {
                    reporter.problem(&format!(
                        "{shown}:{}: error: {}",
                        error.location,
                        single_line(&error.message)
                    ));
                }
            }
            Ok(symbols) => {
                for diagnostic in checker.check(&symbols, &relative) {
                    reporter.problem(&format!(
                        "{shown}:{}: error: {}",
                        diagnostic.location,
                        single_line(&diagnostic.message)
                    ));
                }
            }
        }
    }
}

/// Collects the paths of all files under `read_root`, relative to it and
/// sorted. `.gitignore` (and `.ignore`, `.git/info/exclude`, parent
/// directories' `.gitignore`) are respected even outside a git repository,
/// hidden files such as `.git` are skipped, and symlinks are followed.
/// Walk errors are reported, since silently skipping a directory would hide
/// violations.
fn collect_files(reporter: &mut Reporter, read_root: &Path) -> Vec<PathBuf> {
    let walker = WalkBuilder::new(read_root)
        .follow_links(true)
        .require_git(false)
        .build();
    let mut files = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                reporter.problem(&format!(
                    "{}: error: cannot walk the directory: {error}",
                    read_root.display()
                ));
                continue;
            }
        };
        if !entry.path().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(read_root).unwrap_or(entry.path());
        files.push(relative.to_path_buf());
    }
    files.sort();
    files
}

/// Reads `path` and parses it, reporting every problem through `reporter`.
fn load<T>(
    reporter: &mut Reporter,
    path: &Path,
    parse: fn(&str) -> Result<T, Vec<Diagnostic>>,
) -> Option<T> {
    let shown = path.display();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            reporter.problem(&format!("{shown}: error: cannot read the file: {error}"));
            return None;
        }
    };
    match parse(&source) {
        Ok(value) => Some(value),
        Err(diagnostics) => {
            for diagnostic in &diagnostics {
                reporter.problem(&format!(
                    "{shown}:{}: error: {}",
                    diagnostic.location,
                    single_line(&diagnostic.message)
                ));
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
