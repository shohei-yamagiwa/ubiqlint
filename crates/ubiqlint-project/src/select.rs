//! Choosing the files to check from the paths people and agents give.

use std::path::PathBuf;

use crate::{Project, Skip, WalkError};

/// The files chosen by [`Project::select`].
#[derive(Debug, Default)]
pub struct Selection {
    /// The files to check, relative to the root, sorted and without
    /// duplicates. Files of languages ubiqlint cannot check may be among
    /// those found by walking; [`Project::skip`] tells them apart.
    pub files: Vec<PathBuf>,
    /// The given paths that are not under the root, as given.
    pub outside: Vec<PathBuf>,
    /// The given files that are not checked, relative to the root, in the
    /// order given.
    pub skipped: Vec<(PathBuf, Skip)>,
    /// The paths that could not be walked; see [`crate::Walk`].
    pub walk_errors: Vec<WalkError>,
}

impl Project {
    /// Chooses the files to check: every file under the root if no `paths`
    /// are given, or else the files under the given directories and the
    /// given files themselves. Paths are relative to the current directory
    /// or absolute.
    ///
    /// Directories are walked as the whole tree would be (see
    /// [`Project::walk`]). A file given directly is checked even if
    /// `.gitignore` would hide it from a walk, since that only applies to
    /// finding files, but never if it matches `exclude` or is in a language
    /// ubiqlint cannot check. If every given path is outside the root,
    /// nothing is chosen rather than the whole tree.
    pub fn select(&self, paths: &[PathBuf]) -> Selection {
        let mut selection = Selection::default();
        let mut scopes = Vec::new();
        for path in paths {
            let Some(relative) = self.relative(path) else {
                selection.outside.push(path.clone());
                continue;
            };
            if self.read_root().join(&relative).is_dir() {
                scopes.push(relative);
            } else if let Some(skip) = self.skip(&relative) {
                selection.skipped.push((relative, skip));
            } else {
                selection.files.push(relative);
            }
        }
        if paths.is_empty() || !scopes.is_empty() {
            let walk = self.walk(&scopes);
            selection.walk_errors = walk.errors;
            if paths.is_empty() {
                // The walk is already sorted and has no duplicates.
                selection.files = walk.files;
                return selection;
            }
            selection.files.extend(walk.files);
        }
        selection.files.sort();
        selection.files.dedup();
        selection
    }
}
