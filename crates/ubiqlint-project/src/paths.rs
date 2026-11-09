//! Turning the paths people and agents give into paths under a root.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// Returns `path` relative to `root`, both given relative to the current
/// directory or as absolute paths, or `None` if `path` is not under `root`.
///
/// Paths are compared as written first, after removing `.` and `..`, so
/// that a symlink inside the root keeps its visible path, as dictionary
/// resolution and `include` globs expect. Only if that fails are the real
/// paths compared, for a root reached through a symlink elsewhere (such as
/// a temporary directory) and named differently in `path`.
pub(crate) fn relative(root: &Path, path: &Path) -> Option<PathBuf> {
    if let (Some(root), Some(path)) = (lexical(root), lexical(path)) {
        if let Ok(relative) = path.strip_prefix(&root) {
            return Some(relative.to_owned());
        }
    }
    let root = fs::canonicalize(root).ok()?;
    let path = canonical(path)?;
    path.strip_prefix(&root).ok().map(Path::to_owned)
}

/// Makes `path` absolute and removes `.` and `..` without following
/// symlinks.
fn lexical(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let mut normal = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    Some(normal)
}

/// The real path of `path`, which need not exist as long as its directory
/// does (a file about to be written).
fn canonical(path: &Path) -> Option<PathBuf> {
    if let Ok(path) = fs::canonicalize(path) {
        return Some(path);
    }
    let name = path.file_name()?;
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    Some(fs::canonicalize(parent).ok()?.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_root_from_paths_relative_to_the_current_directory() {
        assert_eq!(
            relative(Path::new("project"), Path::new("project/src/Order.java")),
            Some(PathBuf::from("src/Order.java"))
        );
        assert_eq!(
            relative(Path::new("."), Path::new("src/Order.java")),
            Some(PathBuf::from("src/Order.java"))
        );
        assert_eq!(
            relative(
                Path::new("project"),
                Path::new("./project/src/../lib/Order.java")
            ),
            Some(PathBuf::from("lib/Order.java"))
        );
    }

    #[test]
    fn the_root_itself_is_the_empty_path() {
        assert_eq!(
            relative(Path::new("project"), Path::new("project")),
            Some(PathBuf::new())
        );
        assert_eq!(
            relative(Path::new("."), Path::new(".")),
            Some(PathBuf::new())
        );
    }

    #[test]
    fn accepts_absolute_paths_for_relative_roots_and_the_other_way_round() {
        let current = std::env::current_dir().unwrap();
        assert_eq!(
            relative(Path::new("project"), &current.join("project/Order.java")),
            Some(PathBuf::from("Order.java"))
        );
        assert_eq!(
            relative(&current.join("project"), Path::new("project/Order.java")),
            Some(PathBuf::from("Order.java"))
        );
    }

    #[test]
    fn rejects_paths_outside_the_root() {
        assert_eq!(
            relative(Path::new("project"), Path::new("other/Order.java")),
            None
        );
        assert_eq!(
            relative(Path::new("project"), Path::new("project/../Order.java")),
            None
        );
        // A shared prefix of a name is not a parent directory.
        assert_eq!(
            relative(Path::new("project"), Path::new("projects/Order.java")),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn falls_back_to_real_paths_for_a_root_reached_through_a_symlink() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("ubiqlint-paths-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("real/src")).unwrap();
        symlink(base.join("real"), base.join("link")).unwrap();

        let found = relative(&base.join("link"), &base.join("real/src/New.java"));
        let _ = fs::remove_dir_all(&base);
        assert_eq!(found, Some(PathBuf::from("src/New.java")));
    }
}
