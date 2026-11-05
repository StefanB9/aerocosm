//! `cargo xtask web serve`: a localhost-only static file server for the browser
//! build.

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use super::http::{self, FileSource};

/// Files under a directory; paths are pre-validated by [`http::resolve`].
pub(crate) struct Directory(pub(crate) PathBuf);

impl FileSource for Directory {
    fn read(&self, relative: &Path) -> Option<Vec<u8>> {
        fs::read(self.0.join(relative)).ok()
    }
}

/// Serves `files` on `127.0.0.1:port` until the process is stopped. Connections
/// are handled one after another: plenty for one developer's browser, and no
/// threads to manage.
pub(crate) fn serve(port: u16, files: &Directory) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("cannot listen on 127.0.0.1:{port}"))?;
    if !files.0.join("index.html").is_file() {
        warn!(dir = %files.0.display(), "no index.html; run `cargo xtask web build` first");
    }
    info!(url = %format!("http://localhost:{port}"), dir = %files.0.display(), "serving");
    for connection in listener.incoming() {
        let result = connection.and_then(|mut stream| http::handle(&mut stream, files));
        match result {
            Ok(status) => debug!(status, "request"),
            Err(error) => warn!(%error, "request failed"),
        }
    }
    Ok(())
}
