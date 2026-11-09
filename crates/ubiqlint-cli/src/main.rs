use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ignore::WalkBuilder;
use ubiqlint_core::{
    CONFIG_FILE_NAME, Checker, Config, DICTIONARY_FILE_NAME, Diagnostic, Dictionary, Excludes,
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

    /// Prints one line, returning whether the pipe still accepts writes.
    fn line(&mut self, text: &str) -> bool {
        if self.broken {
            return false;
        }
        match writeln!(self.out, "{text}") {
            Ok(()) => true,
            Err(_) => {
                self.broken = true;
                self.failed = true;
                false
            }
        }
    }

    /// Prints one problem and fails the run.
    fn problem(&mut self, text: &str) {
        self.failed = true;
        self.line(text);
    }

    /// Prints a warning to stderr. Warnings never fail the run and do not
    /// mix into the `path:line:column: error:` lines on stdout.
    fn warn(&mut self, text: &str) {
        let _ = writeln!(io::stderr(), "{text}");
    }

    /// Tells whether the pipe is broken, so callers can stop early instead
    /// of doing a whole run's work into a closed pipe.
    fn broken(&self) -> bool {
        self.broken
    }

    fn finish(mut self) -> ExitCode {
        // A large output can still fail here, when the buffer is flushed.
        if self.out.flush().is_err() {
            self.failed = true;
        }
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
    let excludes = config.excludes_matcher();
    let mut checked = 0;
    for relative in collect_files(reporter, read_root, root, &excludes) {
        if reporter.broken() {
            break;
        }
        if excludes.matches(&relative) {
            continue;
        }
        let Some(adapter) = adapter_for(&relative) else {
            continue;
        };
        checked += 1;
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
    if checked == 0 {
        // Distinguish "no violations" from "nothing was checked", which CI
        // and hooks would otherwise not see. Warnings go to stderr and never
        // fail the run; only problems do.
        reporter.warn("warning: no source files were checked");
    }
}

/// Returns the adapter that handles `path`'s extension, if any.
fn adapter_for(path: &Path) -> Option<&'static dyn LanguageAdapter> {
    let extension = path.extension()?.to_str()?;
    ADAPTERS
        .iter()
        .find(|adapter| adapter.extensions().contains(&extension))
        .copied()
}

/// Collects the paths of all files under `read_root`, relative to it and
/// sorted, displayed by joining them onto `root`. `.gitignore` (and
/// `.ignore`, `.git/info/exclude`) are respected with git's own boundary:
/// inside a repository, `.gitignore` files above it do not apply; outside
/// one (a tarball), only the ones inside the tree do. Hidden files such as
/// `.git` are skipped, symlinks are followed, and directories named by a
/// `/**`-suffixed `exclude` glob are never descended into.
///
/// `exclude` globs apply to files as written; they never prune a directory
/// by matching its name, so `exclude: ["src/*"]` does not swallow
/// `src/domain`.
///
/// Walk errors are reported with the failed path first, since silently
/// skipping a directory would hide violations. Errors under an excluded
/// path, or on files no adapter handles, are not reported: they are not
/// ours to check.
fn collect_files(
    reporter: &mut Reporter,
    read_root: &Path,
    root: &Path,
    excludes: &Excludes,
) -> Vec<PathBuf> {
    let in_repo = read_root.canonicalize().is_ok_and(|dir| {
        dir.ancestors()
            .any(|ancestor| ancestor.join(".git").exists())
    });
    let mut builder = WalkBuilder::new(read_root);
    builder
        .follow_links(true)
        .require_git(in_repo)
        .parents(in_repo);
    let filter_root = read_root.to_path_buf();
    let filter_excludes = excludes.clone();
    builder.filter_entry(move |entry| {
        let relative = entry
            .path()
            .strip_prefix(&filter_root)
            .unwrap_or(entry.path());
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
        {
            !filter_excludes.matches_dir(relative)
        } else {
            !filter_excludes.matches(relative)
        }
    });
    let mut files = Vec::new();
    for entry in builder.build() {
        if reporter.broken() {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                if let Some(path) = error_path(&error) {
                    let relative = path.strip_prefix(read_root).unwrap_or(path);
                    if excludes.matches(relative) {
                        continue;
                    }
                    // A directory that cannot be read may hide violations;
                    // a file that no adapter handles is not ours to check.
                    if !path.is_dir() && adapter_for(path).is_none() {
                        continue;
                    }
                    reporter.problem(&format!(
                        "{}: error: cannot walk: {}",
                        root.join(relative).display(),
                        walk_error_message(&error)
                    ));
                } else {
                    reporter.problem(&format!(
                        "{}: error: cannot walk: {}",
                        read_root.display(),
                        walk_error_message(&error)
                    ));
                }
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

/// Returns the core message of a walk error, without the path the `ignore`
/// crate wraps around it.
fn walk_error_message(error: &ignore::Error) -> String {
    match error {
        ignore::Error::Io(io) => io.to_string(),
        ignore::Error::WithPath { err, .. }
        | ignore::Error::WithLineNumber { err, .. }
        | ignore::Error::WithDepth { err, .. } => walk_error_message(err),
        ignore::Error::Partial(errors) => errors
            .first()
            .map_or_else(|| error.to_string(), walk_error_message),
        other => other.to_string(),
    }
}

/// Returns the path an `ignore::Error` is about, if it has one.
fn error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::Loop { child, .. } => Some(child),
        ignore::Error::Partial(errors) => errors.iter().find_map(error_path),
        ignore::Error::WithLineNumber { err, .. } | ignore::Error::WithDepth { err, .. } => {
            error_path(err)
        }
        _ => None,
    }
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
