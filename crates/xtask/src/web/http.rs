//! A minimal HTTP/1.1 responder for local static files: GET and HEAD only.

use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Request heads longer than this are rejected.
const MAX_HEAD: usize = 16 * 1024;

/// Where served files come from; relative paths are already validated by
/// [`resolve`].
pub(crate) trait FileSource {
    /// The contents of `relative`, or `None` if there is no such file.
    fn read(&self, relative: &Path) -> Option<Vec<u8>>;
}

/// An HTTP response ready to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub(crate) status: u16,
    reason: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) body: Vec<u8>,
    /// `false` for HEAD requests: headers describe the body, but it is not
    /// sent.
    include_body: bool,
}

impl Response {
    fn file(path: &Path, body: Vec<u8>, include_body: bool) -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: content_type(path),
            body,
            include_body,
        }
    }

    fn error(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: format!("{status} {reason}\n").into_bytes(),
            include_body: true,
        }
    }

    fn write_to(&self, out: &mut impl Write) -> io::Result<()> {
        write!(
            out,
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: \
             no-cache\r\nConnection: close\r\n\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        )?;
        if self.include_body {
            out.write_all(&self.body)?;
        }
        out.flush()
    }
}

/// Maps a request target such as `/webgl2/app.js?x=1` to a relative file path,
/// or `None` if it could reach outside the served directory. `/` and paths
/// ending in `/` serve `index.html`. Percent-encoding, backslashes, `.` and
/// `..` are refused outright: the served files never need them, and refusing is
/// simpler than normalising safely.
pub(crate) fn resolve(target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next()?;
    let rest = path.strip_prefix('/')?;
    if rest.starts_with('/') || rest.contains(['\\', '%', '\0']) {
        return None;
    }
    let mut resolved = PathBuf::new();
    let (directories, file) = match rest.rsplit_once('/') {
        Some((directories, file)) => (Some(directories), file),
        None => (None, rest),
    };
    for segment in directories.into_iter().flat_map(|d| d.split('/')) {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
        resolved.push(segment);
    }
    match file {
        "" => resolved.push("index.html"),
        "." | ".." => return None,
        name => resolved.push(name),
    }
    Some(resolved)
}

/// The `Content-Type` for a file; browsers only compile `.wasm` served as
/// `application/wasm`.
pub(crate) fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The response to a request line such as `GET / HTTP/1.1`.
pub(crate) fn respond(request_line: &str, files: &impl FileSource) -> Response {
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Response::error(400, "Bad Request");
    };
    if !version.starts_with("HTTP/") {
        return Response::error(400, "Bad Request");
    }
    let include_body = match method {
        "GET" => true,
        "HEAD" => false,
        _ => return Response::error(405, "Method Not Allowed"),
    };
    let Some(path) = resolve(target) else {
        return Response::error(404, "Not Found");
    };
    match files.read(&path) {
        Some(body) => Response::file(&path, body, include_body),
        None => Response::error(404, "Not Found"),
    }
}

/// Reads one request from `stream`, writes the response and returns its status.
pub(crate) fn handle(stream: &mut (impl Read + Write), files: &impl FileSource) -> io::Result<u16> {
    let head = read_head(stream)?;
    let response = match head {
        Some(head) => respond(head.lines().next().unwrap_or_default(), files),
        None => Response::error(400, "Bad Request"),
    };
    response.write_to(stream)?;
    Ok(response.status)
}

/// The request head up to the blank line, or `None` if it is longer than
/// [`MAX_HEAD`] or not text.
fn read_head(stream: &mut impl Read) -> io::Result<Option<String>> {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if head.len() > MAX_HEAD {
            return Ok(None);
        }
    }
    Ok(String::from_utf8(head).ok())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, io::Cursor};

    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;

    /// In-memory files.
    struct Files(HashMap<PathBuf, Vec<u8>>);

    impl FileSource for Files {
        fn read(&self, relative: &Path) -> Option<Vec<u8>> {
            self.0.get(relative).cloned()
        }
    }

    fn files() -> Files {
        Files(HashMap::from([
            (PathBuf::from("index.html"), b"<html></html>".to_vec()),
            (
                PathBuf::from("webgl2/aerocosm-viewer_bg.wasm"),
                vec![0, 97, 115, 109],
            ),
        ]))
    }

    /// A stream whose reads come from `input` and whose writes collect in
    /// `output`.
    struct Duplex {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_resolve_maps_root_and_directories_to_index() {
        assert_eq!(resolve("/"), Some(PathBuf::from("index.html")));
        assert_eq!(
            resolve("/?renderer=webgl2"),
            Some(PathBuf::from("index.html"))
        );
        assert_eq!(
            resolve("/webgl2/"),
            Some(PathBuf::from("webgl2/index.html"))
        );
        assert_eq!(
            resolve("/webgl2/a.js#x"),
            Some(PathBuf::from("webgl2/a.js"))
        );
    }

    #[test]
    fn test_resolve_rejects_escapes() {
        for target in [
            "/../secret",
            "/a/../../b",
            "relative",
            "/a\\b",
            "/a/./b",
            "//etc/passwd",
            "/%2e%2e/x",
            "",
        ] {
            assert_eq!(resolve(target), None, "{target:?}");
        }
    }

    #[test]
    fn test_content_types() {
        assert_eq!(content_type(Path::new("x.wasm")), "application/wasm");
        assert_eq!(
            content_type(Path::new("x.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("x.unknown")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type(Path::new("noextension")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_respond_serves_files_with_type() {
        let response = respond("GET /webgl2/aerocosm-viewer_bg.wasm HTTP/1.1", &files());
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/wasm");
        assert_eq!(response.body, vec![0, 97, 115, 109]);
    }

    #[test]
    fn test_respond_errors() {
        assert_eq!(respond("GET /missing.js HTTP/1.1", &files()).status, 404);
        assert_eq!(respond("GET /../x HTTP/1.1", &files()).status, 404);
        assert_eq!(respond("POST / HTTP/1.1", &files()).status, 405);
        assert_eq!(respond("garbage", &files()).status, 400);
    }

    #[test]
    fn test_handle_writes_full_response() -> Result<()> {
        let mut stream = Duplex {
            input: Cursor::new(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec()),
            output: Vec::new(),
        };
        let status = handle(&mut stream, &files()).context("handle")?;
        assert_eq!(status, 200);
        let text = String::from_utf8(stream.output).context("utf-8")?;
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(text.contains("Content-Length: 13\r\n"));
        assert!(text.contains("Cache-Control: no-cache\r\n"));
        assert!(text.ends_with("\r\n\r\n<html></html>"));
        Ok(())
    }

    #[test]
    fn test_handle_head_omits_body() -> Result<()> {
        let mut stream = Duplex {
            input: Cursor::new(b"HEAD / HTTP/1.1\r\n\r\n".to_vec()),
            output: Vec::new(),
        };
        handle(&mut stream, &files()).context("handle")?;
        let text = String::from_utf8(stream.output).context("utf-8")?;
        assert!(text.contains("Content-Length: 13\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
        Ok(())
    }

    #[test]
    fn test_handle_rejects_oversized_head() -> Result<()> {
        let mut stream = Duplex {
            input: Cursor::new(vec![b'a'; 20_000]),
            output: Vec::new(),
        };
        assert_eq!(handle(&mut stream, &files()).context("handle")?, 400);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_resolve_never_escapes_the_root(target in "[/a-z.%\\\\?#]{0,30}") {
            if let Some(path) = resolve(&target) {
                prop_assert!(path.is_relative());
                prop_assert!(path.components().all(|c| matches!(c, std::path::Component::Normal(_))));
            }
        }
    }
}
