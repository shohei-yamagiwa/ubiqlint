//! Finding the source files of a project.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::{Project, adapter_for};

/// The files found under a project root.
///
/// `.gitignore` (and `.ignore`, `.git/info/exclude`) are respected with
/// git's own boundary: inside a repository, `.gitignore` files above it do
/// not apply; outside one (a tarball), only the ones inside the tree do.
/// Hidden files such as `.git` are skipped, symlinks are followed, and
/// directories named by a `/**`-suffixed `exclude` glob are never descended
/// into.
///
/// `exclude` globs apply to files as written; they never prune a directory
/// by matching its name, so `exclude: ["src/*"]` does not swallow
/// `src/domain`.
#[derive(Debug, Default)]
pub struct Walk {
    /// Every file found, relative to the root and sorted, whatever its
    /// language.
    pub files: Vec<PathBuf>,
    /// The paths that could not be walked, in the order they were met.
    /// Silently skipping a directory would hide violations, so these are
    /// problems. Errors under an excluded path, or on files no adapter
    /// handles, are left out: they are not ours to check.
    pub errors: Vec<WalkError>,
}

/// A path that could not be walked.
#[derive(Debug)]
pub struct WalkError {
    /// The path relative to the root, or `None` for the root itself.
    pub path: Option<PathBuf>,
    /// What went wrong, without the path.
    pub message: String,
}

pub(crate) fn walk(project: &Project, scopes: &[PathBuf]) -> Walk {
    let read_root = project.read_root();
    let excludes = project.config().excludes();
    let in_repo = read_root.canonicalize().is_ok_and(|dir| {
        dir.ancestors()
            .any(|ancestor| ancestor.join(".git").exists())
    });
    // Walk from the root even for scopes below it, so that the `.gitignore`
    // files between the root and a scope apply just as in a full walk.
    let mut builder = WalkBuilder::new(read_root);
    builder
        .follow_links(true)
        .require_git(in_repo)
        .parents(in_repo);
    let filter_root = read_root.to_path_buf();
    let filter_excludes = excludes.clone();
    let filter_scopes = scopes.to_vec();
    builder.filter_entry(move |entry| {
        let relative = entry
            .path()
            .strip_prefix(&filter_root)
            .unwrap_or(entry.path());
        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir())
        {
            !filter_excludes.matches_dir(relative) && leads_into(&filter_scopes, relative)
        } else {
            !filter_excludes.matches(relative) && within(&filter_scopes, relative)
        }
    });
    let mut walk = Walk::default();
    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                let message = error_message(&error);
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
                    walk.errors.push(WalkError {
                        path: Some(relative.to_path_buf()),
                        message,
                    });
                } else {
                    walk.errors.push(WalkError {
                        path: None,
                        message,
                    });
                }
                continue;
            }
        };
        if !entry.path().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(read_root).unwrap_or(entry.path());
        walk.files.push(relative.to_path_buf());
    }
    walk.files.sort();
    walk
}

/// Whether the file at `relative` is in one of `scopes`. No scopes means
/// the whole tree.
fn within(scopes: &[PathBuf], relative: &Path) -> bool {
    scopes.is_empty() || scopes.iter().any(|scope| relative.starts_with(scope))
}

/// Whether the directory at `relative` is in one of `scopes`, or on the way
/// to one.
fn leads_into(scopes: &[PathBuf], relative: &Path) -> bool {
    scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| relative.starts_with(scope) || scope.starts_with(relative))
}

/// Returns the core message of a walk error, without the path wrappers:
/// walkdir wraps its own error in an `io::Error` whose text repeats the
/// path ("IO error for operation on ..."), so unwrap to the raw OS error.
fn error_message(error: &ignore::Error) -> String {
    match error {
        ignore::Error::Io(io) => io
            .get_ref()
            .and_then(|inner| inner.source())
            .map_or_else(|| io.to_string(), ToString::to_string),
        ignore::Error::WithPath { err, .. }
        | ignore::Error::WithLineNumber { err, .. }
        | ignore::Error::WithDepth { err, .. } => error_message(err),
        ignore::Error::Partial(errors) => errors
            .first()
            .map_or_else(|| error.to_string(), error_message),
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
