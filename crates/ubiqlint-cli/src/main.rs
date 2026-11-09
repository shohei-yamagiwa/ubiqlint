use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ignore::WalkBuilder;
use ubiqlint_core::{
    CONFIG_FILE_NAME, Checker, Config, DICTIONARY_FILE_NAME, Diagnostic, Dictionary,
    DictionaryResolver, Excludes, LanguageAdapter, ResolutionError,
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
        /// Use this dictionary for every source file instead of resolving
        /// the nearest `terms.yaml` up to the configuration directory
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
    reported: HashSet<PathBuf>,
}

impl Reporter {
    fn new() -> Self {
        Self {
            out: io::BufWriter::new(io::stdout().lock()),
            failed: false,
            broken: false,
            reported: HashSet::new(),
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

    /// Formats all parser and checker diagnostics consistently.
    fn diagnostics(&mut self, path: &Path, diagnostics: &[Diagnostic]) {
        for diagnostic in diagnostics {
            self.problem(&format!(
                "{}:{}: error: {}",
                path.display(),
                diagnostic.location,
                single_line(&diagnostic.message)
            ));
        }
    }

    /// Reports a dictionary failure once, even if many files use it.
    fn resolution_error(&mut self, error: &ResolutionError) {
        if !self.reported.insert(error.path().to_owned()) {
            return;
        }
        match error {
            ResolutionError::InvalidDictionary { path, diagnostics } => {
                self.diagnostics(path, diagnostics);
            }
            _ => self.problem(&format!(
                "{}: error: {}",
                error.path().display(),
                single_line(&error.to_string())
            )),
        }
    }

    /// A single missing dictionary can affect many files in a directory.
    fn missing_dictionary(&mut self, directory: &Path) {
        let directory = if directory.as_os_str().is_empty() {
            Path::new(".")
        } else {
            directory
        };
        if self.reported.insert(directory.to_owned()) {
            self.problem(&format!(
                "{}: error: no `{DICTIONARY_FILE_NAME}` found from this directory \
                 up to the configuration directory; add a dictionary or use `--dictionary`",
                directory.display()
            ));
        }
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
    // Every checked path is relative to the configuration directory, which
    // is also the upper boundary of dictionary resolution.
    let root = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new(""));
    let read_root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    // Validate the root dictionary, if present, alongside the config so
    // both sets of definition errors are still reported in one run. A root
    // dictionary is optional: a project may only have nested contexts.
    let config = load(reporter, config_path, Config::from_yaml_str);
    let mut resolver = DictionaryResolver::new(root);
    let fixed_dictionary = if let Some(path) = dictionary {
        let loaded = load(reporter, path, Dictionary::from_yaml_str);
        if loaded.is_none() {
            return;
        }
        loaded
    } else {
        if let Err(error) = resolver.root_dictionary() {
            reporter.resolution_error(&error);
        }
        None
    };
    let Some(config) = config else {
        return;
    };

    let excludes = config.excludes();
    let files = collect_files(reporter, read_root, root, excludes);
    // Finish dictionary loading before borrowing any checkers. This lets
    // every source using a dictionary share its lookup indices. Only paths
    // covered by rules need resolution; errors are reported later, after
    // the corresponding source has passed its read and syntax checks.
    let mut resolutions = HashMap::new();
    if fixed_dictionary.is_none() {
        for relative in &files {
            if excludes.matches(relative)
                || adapter_for(relative).is_none()
                || !config.rules.iter().any(|rule| rule.includes(relative))
            {
                continue;
            }
            let directory = relative
                .parent()
                .expect("a source path has a parent directory");
            resolutions.entry(directory).or_insert_with(|| {
                resolver
                    .resolve(relative)
                    .map(|resolved| resolved.map(|resolved| resolved.path.to_owned()))
            });
        }
    }
    let mut checkers = HashMap::new();
    let mut checked = 0;
    for relative in &files {
        if reporter.broken() {
            break;
        }
        if excludes.matches(relative) {
            continue;
        }
        let Some(adapter) = adapter_for(relative) else {
            continue;
        };
        checked += 1;
        let path = root.join(relative);
        let Some(source) = read_file(reporter, &path) else {
            continue;
        };
        let symbols = match adapter.extract(&source) {
            Ok(symbols) => symbols,
            Err(diagnostics) => {
                reporter.diagnostics(&path, &diagnostics);
                continue;
            }
        };
        // Files outside every rule still get syntax checks, as before, but
        // do not need a domain dictionary merely for being in the tree.
        if !config.rules.iter().any(|rule| rule.includes(relative)) {
            continue;
        }
        let (dictionary_path, resolved_dictionary) = if let Some(fixed) = &fixed_dictionary {
            (
                dictionary.expect("an explicit dictionary was loaded"),
                fixed,
            )
        } else {
            match resolutions
                .get(
                    relative
                        .parent()
                        .expect("a source path has a parent directory"),
                )
                .expect("every applicable source directory was resolved")
            {
                Ok(Some(path)) => (
                    path.as_path(),
                    resolver
                        .loaded_dictionary(path)
                        .expect("a resolved dictionary was loaded successfully"),
                ),
                Ok(None) => {
                    reporter.missing_dictionary(
                        path.parent().expect("a source file has a parent directory"),
                    );
                    continue;
                }
                Err(error) => {
                    reporter.resolution_error(error);
                    continue;
                }
            }
        };
        let checker = checkers
            .entry(dictionary_path)
            .or_insert_with(|| Checker::new(resolved_dictionary, dictionary_path, &config));
        let diagnostics = checker.check(&symbols, relative);
        reporter.diagnostics(&path, &diagnostics);
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

/// Returns the core message of a walk error, without the path wrappers:
/// walkdir wraps its own error in an `io::Error` whose text repeats the
/// path ("IO error for operation on ..."), so unwrap to the raw OS error.
fn walk_error_message(error: &ignore::Error) -> String {
    match error {
        ignore::Error::Io(io) => io
            .get_ref()
            .and_then(|inner| inner.source())
            .map_or_else(|| io.to_string(), ToString::to_string),
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
    let source = read_file(reporter, path)?;
    match parse(&source) {
        Ok(value) => Some(value),
        Err(diagnostics) => {
            reporter.diagnostics(path, &diagnostics);
            None
        }
    }
}

/// Reads input files using the same error format for definitions and source code.
fn read_file(reporter: &mut Reporter, path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(source) => Some(source),
        Err(error) => {
            reporter.problem(&format!(
                "{}: error: cannot read the file: {error}",
                path.display()
            ));
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
