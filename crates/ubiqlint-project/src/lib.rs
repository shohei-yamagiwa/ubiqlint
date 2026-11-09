//! The checks of a project, shared by the command line and the servers.
//!
//! A [`Project`] is a configuration file together with its directory, the
//! root of every path it checks. It finds the source files under the root
//! ([`Project::walk`]), checks the text of a source file against its rules
//! and its nearest dictionary ([`CheckRun::check`]), and writes edited
//! dictionaries back to disk ([`edit_dictionary`]).
//!
//! Results are values, not messages: the command line, the MCP server and
//! the LSP server each present them in their own way. Dictionaries and the
//! configuration are always read from disk, while the text of a source file
//! is given, so that a draft or an unsaved buffer is checked the same way as
//! a file.

mod paths;
mod walk;
mod write;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ubiqlint_core::{
    Checker, Config, Diagnostic, Dictionary, DictionaryResolver, LanguageAdapter, ResolutionError,
};
use ubiqlint_java::JavaAdapter;

pub use walk::{Walk, WalkError};
pub use write::{EditFileError, EditOutcome, edit_dictionary, replace_file};

/// The adapters for the languages ubiqlint can check.
const ADAPTERS: [&dyn LanguageAdapter; 1] = [&JavaAdapter];

/// Returns the adapter that handles `path`'s extension, if any.
pub fn adapter_for(path: &Path) -> Option<&'static dyn LanguageAdapter> {
    let extension = path.extension()?.to_str()?;
    ADAPTERS
        .iter()
        .find(|adapter| adapter.extensions().contains(&extension))
        .copied()
}

/// Why a definition file could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The file could not be read.
    Read(io::Error),
    /// The file was read but is not a valid definition.
    Invalid(Vec<Diagnostic>),
}

/// Reads the file at `path` and parses it with `parse`, such as
/// [`Config::from_yaml_str`] or [`Dictionary::from_yaml_str`].
pub fn load<T>(path: &Path, parse: fn(&str) -> Result<T, Vec<Diagnostic>>) -> Result<T, LoadError> {
    let source = fs::read_to_string(path).map_err(LoadError::Read)?;
    parse(&source).map_err(LoadError::Invalid)
}

/// Returns the directory of the configuration file at `config_path`, the
/// root of its project. It is empty for a file in the current directory.
pub fn config_root(config_path: &Path) -> &Path {
    config_path.parent().unwrap_or_else(|| Path::new(""))
}

/// A configuration file and the directory it applies to.
#[derive(Debug)]
pub struct Project {
    root: PathBuf,
    config: Config,
}

/// Why a file is not checked at all. Such a file is never read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// The file matches `exclude`.
    Excluded,
    /// No language adapter handles the file's extension.
    UnsupportedLanguage,
}

/// The dictionaries a check uses.
#[derive(Debug)]
pub enum Dictionaries {
    /// One dictionary for every source file, identified by `path` in
    /// diagnostics.
    Fixed {
        path: PathBuf,
        dictionary: Dictionary,
    },
    /// The nearest `terms.yaml` of each source file.
    Nearest(DictionaryResolver),
}

/// The result of checking one source file.
#[derive(Clone, Debug)]
pub enum FileReport {
    /// The file was not checked at all.
    Skipped(Skip),
    /// The file could not be parsed, so it was not checked against what
    /// the parser guessed.
    SyntaxErrors(Vec<Diagnostic>),
    /// The file was parsed, but no rule applies to it.
    NoRules,
    /// No dictionary was found from the file's directory up to the root.
    MissingDictionary,
    /// The nearest dictionary could not be read or is invalid.
    InvalidDictionary(ResolutionError),
    /// The file was checked. The violations, if any, are in source order.
    Checked(Vec<Diagnostic>),
}

impl Project {
    /// Loads the configuration file at `config_path`.
    pub fn open(config_path: &Path) -> Result<Self, LoadError> {
        let config = load(config_path, Config::from_yaml_str)?;
        Ok(Self::new(config_root(config_path), config))
    }

    /// Creates a project for `config`, which applies to `root`.
    pub fn new(root: impl Into<PathBuf>, config: Config) -> Self {
        Self {
            root: root.into(),
            config,
        }
    }

    /// The directory of the configuration file, as given. Paths in
    /// diagnostics join it with paths relative to it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The root as a path that can be read: `.` rather than an empty path.
    pub fn read_root(&self) -> &Path {
        if self.root.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.root
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Tells why the file at `relative`, a path relative to the root, is
    /// not checked at all, or returns `None` if it is a source to check.
    pub fn skip(&self, relative: &Path) -> Option<Skip> {
        if self.config.excludes().matches(relative) {
            Some(Skip::Excluded)
        } else if adapter_for(relative).is_none() {
            Some(Skip::UnsupportedLanguage)
        } else {
            None
        }
    }

    /// Returns `path`, given relative to the current directory or as an
    /// absolute path, relative to the root. Returns `None` if `path` is not
    /// under the root. The root itself is the empty path.
    pub fn relative(&self, path: &Path) -> Option<PathBuf> {
        paths::relative(self.read_root(), path)
    }

    /// Finds the source files under the root, or only under `scopes`,
    /// directories relative to the root, if any are given. See
    /// [`Walk`] for what is skipped.
    pub fn walk(&self, scopes: &[PathBuf]) -> Walk {
        walk::walk(self, scopes)
    }

    /// Returns dictionaries resolved to the nearest `terms.yaml` of each
    /// source file, up to the root.
    pub fn nearest_dictionaries(&self) -> Dictionaries {
        Dictionaries::Nearest(DictionaryResolver::new(&self.root))
    }
}

/// Checks source files of a project, sharing the loaded dictionaries and
/// their lookup indices between files.
///
/// The dictionaries of every source are resolved when the run is created,
/// so only the sources given then can be checked. Create a new run, with
/// new [`Dictionaries`], after a dictionary changes on disk.
pub struct CheckRun<'p> {
    project: &'p Project,
    dictionaries: &'p Dictionaries,
    /// The resolved dictionary path of each source directory.
    resolutions: HashMap<PathBuf, Result<Option<PathBuf>, ResolutionError>>,
    checkers: HashMap<&'p Path, Checker<'p>>,
}

impl<'p> CheckRun<'p> {
    /// Prepares to check `sources`, paths relative to the project root, by
    /// loading the dictionaries they use. Problems with the dictionaries
    /// are reported when a source using them is checked.
    pub fn new<'s>(
        project: &'p Project,
        dictionaries: &'p mut Dictionaries,
        sources: impl IntoIterator<Item = &'s Path>,
    ) -> Self {
        let mut resolutions = HashMap::new();
        if let Dictionaries::Nearest(resolver) = &mut *dictionaries {
            // Only the sources that rules apply to need a dictionary.
            for relative in sources {
                let directory = directory_of(relative);
                if resolutions.contains_key(directory)
                    || project.skip(relative).is_some()
                    || !project.config.has_rules_for(relative)
                {
                    continue;
                }
                let resolution = resolver
                    .resolve(relative)
                    .map(|resolved| resolved.map(|resolved| resolved.path.to_owned()));
                resolutions.insert(directory.to_owned(), resolution);
            }
        }
        Self {
            project,
            dictionaries,
            resolutions,
            checkers: HashMap::new(),
        }
    }

    /// Checks `text` as the contents of the source file at `relative`, a
    /// path relative to the project root that was given to [`Self::new`].
    ///
    /// The file is parsed before its dictionary is looked at, so that a
    /// file with syntax errors is reported for them alone.
    ///
    /// # Panics
    ///
    /// Panics if `relative` needs a dictionary but was not given to
    /// [`Self::new`].
    pub fn check(&mut self, relative: &Path, text: &str) -> FileReport {
        let project = self.project;
        if let Some(skip) = project.skip(relative) {
            return FileReport::Skipped(skip);
        }
        let adapter = adapter_for(relative).expect("a source that is not skipped has an adapter");
        let file = match adapter.extract(text) {
            Ok(file) => file,
            Err(diagnostics) => return FileReport::SyntaxErrors(diagnostics),
        };
        if !project.config.has_rules_for(relative) {
            return FileReport::NoRules;
        }
        let (path, dictionary) = match self.dictionaries {
            Dictionaries::Fixed { path, dictionary } => (path.as_path(), dictionary),
            Dictionaries::Nearest(resolver) => match self
                .resolutions
                .get(directory_of(relative))
                .expect("a checked source was given when the run was created")
            {
                Ok(Some(path)) => {
                    let resolved = resolver
                        .loaded_dictionary(path)
                        .expect("a resolved dictionary was loaded successfully");
                    (resolved.path, resolved.dictionary)
                }
                Ok(None) => return FileReport::MissingDictionary,
                Err(error) => return FileReport::InvalidDictionary(error.clone()),
            },
        };
        let checker = self
            .checkers
            .entry(path)
            .or_insert_with(|| Checker::new(dictionary, path, &project.config));
        FileReport::Checked(checker.check_file(&file, relative))
    }
}

/// The directory a source file resolves its dictionary from.
fn directory_of(relative: &Path) -> &Path {
    relative.parent().unwrap_or_else(|| Path::new(""))
}
