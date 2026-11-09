//! Local-file URI conversion shared by the protocol and session layers.

use std::path::{Path, PathBuf};

use lsp_types::Uri;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

use crate::Result;

const URI_PATH_ESCAPES: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b']')
    .add(b'\\')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub(crate) fn file_path(uri: &Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

pub(crate) fn directory_uri(path: &Path) -> Result<Uri> {
    let url = url::Url::from_directory_path(path)
        .map_err(|()| "the configuration directory must be absolute")?;
    // WHATWG file URLs can leave characters such as brackets unescaped;
    // lsp_types::Uri expects RFC 3986. Escape only the path, preserving
    // existing percent escapes and the authority (including IPv6 brackets).
    Ok(format!(
        "{}{}",
        &url[..url::Position::BeforePath],
        utf8_percent_encode(url.path(), URI_PATH_ESCAPES)
    )
    .parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_directory_paths_as_rfc_uris_without_double_escaping() {
        let path = std::env::temp_dir().join("注文 [draft]^ 100%");
        let uri = directory_uri(&path).unwrap();
        assert!(uri.as_str().contains("%5Bdraft%5D%5E%20100%25"));
        assert_eq!(file_path(&uri).unwrap(), path);
    }
}
