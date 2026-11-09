//! Writing edited dictionaries back to disk.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ubiqlint_core::edit::{self, Edit, EditError, Outcome};

/// What [`edit_dictionary`] did to the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOutcome {
    /// The dictionary was rewritten.
    Changed,
    /// The dictionary already had the requested values, so it was left as
    /// it was.
    Unchanged,
}

/// Why [`edit_dictionary`] left the file as it was.
#[derive(Debug)]
pub enum EditFileError {
    /// The dictionary could not be read.
    Read(io::Error),
    /// The edit was refused; see [`EditError`].
    Edit(EditError),
    /// The edited dictionary could not be written. The file is unchanged.
    Write(io::Error),
}

/// Applies `edit` to the dictionary at `path` and writes the result back
/// with [`replace_file`]. The file changes only if the edit succeeds.
pub fn edit_dictionary(path: &Path, edit: &Edit) -> Result<EditOutcome, EditFileError> {
    let source = fs::read_to_string(path).map_err(EditFileError::Read)?;
    match edit::apply(&source, edit).map_err(EditFileError::Edit)? {
        Outcome::Changed(text) => {
            replace_file(path, &text).map_err(EditFileError::Write)?;
            Ok(EditOutcome::Changed)
        }
        Outcome::Unchanged => Ok(EditOutcome::Unchanged),
    }
}

/// Replaces the contents of the file at `path` all at once: the text is
/// written to a temporary file next to it, which then takes its place. A
/// failure midway leaves the old file untouched. A symlink keeps pointing
/// to the same file, which keeps its permissions. If something already
/// exists at the temporary path, nothing is written and it is left alone.
pub fn replace_file(path: &Path, contents: &str) -> io::Result<()> {
    let target = fs::canonicalize(path)?;
    let permissions = fs::metadata(&target)?.permissions();
    let temporary = temporary_path(&target)?;
    // Create the file only if nothing is there: an existing path, perhaps
    // a symlink to an unrelated file, must not be written through.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "cannot create the temporary file `{}`: {error}",
                    temporary.display()
                ),
            )
        })?;
    let written = (|| {
        file.write_all(contents.as_bytes())?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        fs::rename(&temporary, &target)
    })();
    // Only the file created above is cleaned up.
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// The temporary file `replace_file` writes before replacing `target`: a
/// hidden file next to it, named after it and this process.
fn temporary_path(target: &Path) -> io::Result<PathBuf> {
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file"))?;
    let mut temporary = std::ffi::OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".ubiqlint-{}.tmp", std::process::id()));
    Ok(target.with_file_name(temporary))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file already at the temporary path is neither written through
    /// (it may be a symlink to an unrelated file) nor removed, and the
    /// dictionary stays as it was.
    #[test]
    fn replace_file_leaves_an_existing_temporary_path_alone() {
        let directory =
            std::env::temp_dir().join(format!("ubiqlint-replace-file-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let dictionary = directory.join("terms.yaml");
        fs::write(&dictionary, "old").unwrap();
        let temporary = temporary_path(&fs::canonicalize(&dictionary).unwrap()).unwrap();

        fs::write(&temporary, "someone else's").unwrap();
        assert!(replace_file(&dictionary, "new").is_err());
        assert_eq!(fs::read_to_string(&temporary).unwrap(), "someone else's");
        assert_eq!(fs::read_to_string(&dictionary).unwrap(), "old");

        #[cfg(unix)]
        {
            let unrelated = directory.join("unrelated.txt");
            fs::write(&unrelated, "unrelated").unwrap();
            fs::remove_file(&temporary).unwrap();
            std::os::unix::fs::symlink(&unrelated, &temporary).unwrap();
            assert!(replace_file(&dictionary, "new").is_err());
            assert_eq!(fs::read_to_string(&unrelated).unwrap(), "unrelated");
            assert!(
                fs::symlink_metadata(&temporary)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert!(
                fs::symlink_metadata(&dictionary)
                    .unwrap()
                    .file_type()
                    .is_file()
            );
            assert_eq!(fs::read_to_string(&dictionary).unwrap(), "old");
        }
        fs::remove_dir_all(&directory).unwrap();
    }
}
