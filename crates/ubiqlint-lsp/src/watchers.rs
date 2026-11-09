//! Portable definition-file watchers, negotiated with the client.

use std::path::Path;

use lsp_types::{
    DidChangeWatchedFilesRegistrationOptions, FileSystemWatcher, GlobPattern, OneOf,
    RelativePattern,
};

use crate::{Result, uri::directory_uri};

pub(crate) fn options(
    config: &Path,
    relative: bool,
) -> Result<DidChangeWatchedFilesRegistrationOptions> {
    let base = if relative {
        Some(directory_uri(ubiqlint_project::config_root(config))?)
    } else {
        None
    };
    // If the name contains glob syntax, watch this directory's files instead
    // of guessing a client's escaping rules. Session filters the actual path.
    let config_pattern = config
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.contains(['*', '?', '[', ']', '{', '}', '\\']))
        .unwrap_or("*");
    let watchers = ["**/terms.yaml", config_pattern]
        .into_iter()
        .map(|pattern| {
            let glob_pattern = if let Some(base) = &base {
                GlobPattern::Relative(RelativePattern {
                    base_uri: OneOf::Right(base.clone()),
                    pattern: pattern.to_owned(),
                })
            } else {
                // A filename glob avoids drive-letter absolute patterns on Windows.
                // The session ignores matching config names in other directories.
                GlobPattern::String(if pattern.starts_with("**/") {
                    pattern.to_owned()
                } else {
                    format!("**/{pattern}")
                })
            };
            FileSystemWatcher {
                glob_pattern,
                kind: None,
            }
        })
        .collect();
    Ok(DidChangeWatchedFilesRegistrationOptions { watchers })
}
