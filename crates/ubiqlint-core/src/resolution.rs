//! Resolving a source file to the nearest dictionary within a project.
//!
//! A dictionary starts a bounded context: it replaces, rather than merges
//! with, any ancestor dictionary. Paths are lexical, so a symlink uses the
//! dictionaries alongside its visible path, just like configuration globs.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::{DICTIONARY_FILE_NAME, Diagnostic, Dictionary};

/// The dictionary selected for a source file, including its definition path.
#[derive(Debug)]
pub struct ResolvedDictionary<'a> {
    pub path: &'a Path,
    pub dictionary: &'a Dictionary,
}

/// A dictionary could not be read or parsed, or the source path escapes the root.
#[derive(Clone, Debug)]
pub enum ResolutionError {
    InvalidPath {
        path: PathBuf,
    },
    Read {
        path: PathBuf,
        error: Arc<io::Error>,
    },
    InvalidDictionary {
        path: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
}

impl ResolutionError {
    /// The source or dictionary path to which the error belongs.
    pub fn path(&self) -> &Path {
        match self {
            Self::InvalidPath { path }
            | Self::Read { path, .. }
            | Self::InvalidDictionary { path, .. } => path,
        }
    }
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath { .. } => {
                write!(
                    f,
                    "expected a source file path relative to the resolution root"
                )
            }
            Self::Read { error, .. } => write!(f, "cannot read the file: {error}"),
            Self::InvalidDictionary { .. } => write!(f, "invalid dictionary"),
        }
    }
}

impl std::error::Error for ResolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}

/// Finds and loads dictionaries for files relative to a configuration root.
///
/// Searches from each source file's directory up to and including `root`,
/// never above it. The nearest `terms.yaml` is the entire context, even when
/// empty. An unreadable or invalid dictionary is an error, never a reason to
/// fall back to an ancestor. No dictionary is represented by `Ok(None)`.
///
/// Dictionary lookups are dependencies of source files, independent of file
/// discovery's ignore/exclude rules. Dictionaries, missing candidates and
/// failures are cached for one run. Create a new resolver after editing files.
#[derive(Debug)]
pub struct DictionaryResolver {
    root: PathBuf,
    dictionaries: HashMap<PathBuf, Result<Option<Dictionary>, ResolutionError>>,
}

impl DictionaryResolver {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            dictionaries: HashMap::new(),
        }
    }

    /// Loads the optional dictionary at the configuration root itself.
    pub fn root_dictionary(&mut self) -> Result<Option<ResolvedDictionary<'_>>, ResolutionError> {
        self.resolve(Path::new(DICTIONARY_FILE_NAME))
    }

    /// Borrows a successfully loaded dictionary by its resolved path,
    /// without any filesystem access. Resolve the source paths first, then
    /// use shared borrows to keep one checker per dictionary for the run.
    pub fn loaded_dictionary(&self, path: &Path) -> Option<&Dictionary> {
        match self.dictionaries.get(path) {
            Some(Ok(Some(dictionary))) => Some(dictionary),
            _ => None,
        }
    }

    /// Resolves `source`, a file path relative to the configuration root.
    /// The source file need not exist (for example, an editor's new buffer).
    /// Absolute paths and paths escaping the root are rejected.
    pub fn resolve(
        &mut self,
        source: &Path,
    ) -> Result<Option<ResolvedDictionary<'_>>, ResolutionError> {
        let relative = relative_file(source).ok_or_else(|| ResolutionError::InvalidPath {
            path: source.to_owned(),
        })?;
        let mut directory = relative
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_owned();
        let selected = loop {
            let candidate = self.root.join(&directory).join(DICTIONARY_FILE_NAME);
            let result = self
                .dictionaries
                .entry(candidate.clone())
                .or_insert_with(|| read_dictionary(&candidate));
            match result {
                Ok(Some(_)) => break candidate,
                Err(error) => return Err(error.clone()),
                Ok(None) => {}
            }
            if !directory.pop() {
                return Ok(None);
            }
        };
        let (path, result) = self.dictionaries.get_key_value(&selected).unwrap();
        let Ok(Some(dictionary)) = result else {
            unreachable!("the selected dictionary was loaded successfully")
        };
        Ok(Some(ResolvedDictionary { path, dictionary }))
    }
}

/// Normalizes a relative file path without following symlinks.
fn relative_file(path: &Path) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir if relative.pop() => {}
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

fn read_dictionary(path: &Path) -> Result<Option<Dictionary>, ResolutionError> {
    let read_error = |error| ResolutionError::Read {
        path: path.to_owned(),
        error: Arc::new(error),
    };
    // A dangling symlink is a broken dictionary, not an absent one. Inspect
    // the link itself before reading, so it cannot silently inherit a parent.
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(read_error(error)),
    }
    let source = fs::read_to_string(path).map_err(read_error)?;
    Dictionary::from_yaml_str(&source)
        .map(Some)
        .map_err(|diagnostics| ResolutionError::InvalidDictionary {
            path: path.to_owned(),
            diagnostics,
        })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

    struct Project(PathBuf);

    impl Project {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ubiqlint-resolver-{}-{}",
                std::process::id(),
                NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, path: &str, source: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dictionary(context: &str, noun: &str) -> String {
        format!(
            "context: {context}\nterms:\n  - name: {noun}\n    forms: {{ singular: {}, plural: {}s }}\n",
            noun.to_ascii_lowercase(),
            noun.to_ascii_lowercase()
        )
    }

    #[test]
    fn selects_the_nearest_dictionary_without_merging_ancestors() {
        let project = Project::new();
        project.write("terms.yaml", &dictionary("root", "Order"));
        project.write("src/terms.yaml", &dictionary("local", "Invoice"));
        let mut resolver = DictionaryResolver::new(&project.0);

        for source in ["src/Invoice.java", "src/domain/Invoice.java"] {
            let resolved = resolver.resolve(Path::new(source)).unwrap().unwrap();
            assert_eq!(resolved.path, project.0.join("src/terms.yaml"));
            assert_eq!(resolved.dictionary.context, "local");
            assert_eq!(resolved.dictionary.terms.len(), 1);
            assert_eq!(resolved.dictionary.terms[0].name, "Invoice");
        }
        let resolved = resolver
            .resolve(Path::new("test/Order.java"))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.path, project.0.join("terms.yaml"));
        assert_eq!(resolved.dictionary.context, "root");
    }

    #[test]
    fn separates_sibling_contexts_even_with_the_same_context_and_term_names() {
        let project = Project::new();
        project.write("order/terms.yaml", &dictionary("sales", "Order"));
        project.write("billing/terms.yaml", &dictionary("sales", "Order"));
        let mut resolver = DictionaryResolver::new(&project.0);

        assert!(resolver.root_dictionary().unwrap().is_none());
        for directory in ["order", "billing"] {
            let source = PathBuf::from(directory).join("domain/Order.java");
            let resolved = resolver.resolve(&source).unwrap().unwrap();
            assert_eq!(resolved.path, project.0.join(directory).join("terms.yaml"));
        }
        assert!(
            resolver
                .resolve(Path::new("other/Order.java"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn an_empty_dictionary_still_starts_a_context() {
        let project = Project::new();
        project.write("terms.yaml", &dictionary("root", "Order"));
        project.write("src/terms.yaml", "context: empty\nterms: []\n");
        let mut resolver = DictionaryResolver::new(&project.0);

        let resolved = resolver
            .resolve(Path::new("src/Order.java"))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.dictionary.context, "empty");
        assert!(resolved.dictionary.terms.is_empty());
    }

    #[test]
    fn never_searches_above_the_config_root() {
        let project = Project::new();
        project.write("terms.yaml", &dictionary("outside", "Order"));
        project.write("project/placeholder", "");
        let mut resolver = DictionaryResolver::new(project.0.join("project"));

        assert!(
            resolver
                .resolve(Path::new("src/Order.java"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_absolute_and_escaping_source_paths_but_normalizes_relative_ones() {
        let project = Project::new();
        project.write("terms.yaml", &dictionary("root", "Order"));
        let mut resolver = DictionaryResolver::new(&project.0);

        for source in [
            PathBuf::from("../Order.java"),
            PathBuf::from("src/../../Order.java"),
            PathBuf::new(),
            PathBuf::from("."),
            project.0.join("Order.java"),
        ] {
            assert!(matches!(
                resolver.resolve(&source),
                Err(ResolutionError::InvalidPath { .. })
            ));
        }
        let resolved = resolver
            .resolve(Path::new("./src/../Order.java"))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.path, project.0.join("terms.yaml"));
    }

    #[test]
    fn invalid_and_unreadable_dictionaries_never_fall_back() {
        let project = Project::new();
        project.write("terms.yaml", &dictionary("root", "Order"));
        project.write("bad/terms.yaml", "context: bad\nterms: wrong\n");
        fs::create_dir_all(project.0.join("unreadable/terms.yaml")).unwrap();
        let mut resolver = DictionaryResolver::new(&project.0);

        let error = resolver.resolve(Path::new("bad/Order.java")).unwrap_err();
        assert_eq!(error.path(), project.0.join("bad/terms.yaml"));
        let ResolutionError::InvalidDictionary { diagnostics, .. } = error else {
            panic!("expected dictionary diagnostics");
        };
        assert_eq!(diagnostics[0].location.line, 2);
        assert!(matches!(
            resolver.resolve(Path::new("unreadable/Order.java")),
            Err(ResolutionError::Read { .. })
        ));
        assert!(
            resolver
                .resolve(Path::new("ok/Order.java"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn caches_definitions_absences_and_errors_for_one_run() {
        let project = Project::new();
        project.write("terms.yaml", &dictionary("root", "Order"));
        project.write("bad/terms.yaml", "terms: wrong\n");
        let mut resolver = DictionaryResolver::new(&project.0);
        resolver.resolve(Path::new("src/Order.java")).unwrap();
        assert!(resolver.resolve(Path::new("bad/Order.java")).is_err());

        project.write("terms.yaml", &dictionary("changed", "Invoice"));
        project.write("src/terms.yaml", &dictionary("new", "Invoice"));
        project.write("bad/terms.yaml", &dictionary("repaired", "Order"));
        let resolved = resolver
            .resolve(Path::new("src/Other.java"))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.dictionary.context, "root");
        assert!(resolver.resolve(Path::new("bad/Other.java")).is_err());

        let mut next_run = DictionaryResolver::new(&project.0);
        let resolved = next_run
            .resolve(Path::new("src/Other.java"))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.dictionary.context, "new");
        let resolved = next_run
            .resolve(Path::new("bad/Other.java"))
            .unwrap()
            .unwrap();
        assert_eq!(resolved.dictionary.context, "repaired");
    }

    #[test]
    fn loaded_dictionaries_can_be_borrowed_together_after_resolution() {
        let project = Project::new();
        project.write("terms.yaml", &dictionary("root", "Order"));
        project.write("src/terms.yaml", &dictionary("nested", "Invoice"));
        let mut resolver = DictionaryResolver::new(&project.0);
        let root_path = resolver
            .resolve(Path::new("Order.java"))
            .unwrap()
            .unwrap()
            .path
            .to_owned();
        let nested_path = resolver
            .resolve(Path::new("src/Invoice.java"))
            .unwrap()
            .unwrap()
            .path
            .to_owned();

        let root = resolver.loaded_dictionary(&root_path).unwrap();
        let nested = resolver.loaded_dictionary(&nested_path).unwrap();
        assert_eq!(root.context, "root");
        assert_eq!(nested.context, "nested");
        assert!(
            resolver
                .loaded_dictionary(&project.0.join("missing/terms.yaml"))
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_dictionary_symlink_is_an_error() {
        use std::os::unix::fs::symlink;

        let project = Project::new();
        project.write("terms.yaml", &dictionary("root", "Order"));
        fs::create_dir_all(project.0.join("src")).unwrap();
        symlink("missing", project.0.join("src/terms.yaml")).unwrap();
        let mut resolver = DictionaryResolver::new(&project.0);

        let error = resolver.resolve(Path::new("src/Order.java")).unwrap_err();
        assert_eq!(error.path(), project.0.join("src/terms.yaml"));
        assert!(matches!(error, ResolutionError::Read { .. }));
    }
}
