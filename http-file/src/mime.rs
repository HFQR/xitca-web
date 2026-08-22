//! content type guessing from a file extension.
//!
//! a full IANA database costs several dependencies and a large generated table to cover
//! thousands of extensions that never appear in http traffic. the common web types are
//! matched here instead and anything unknown falls back to `application/octet-stream`.
//!
//! see [ServeDir::mime_fn] for overriding or extending this with an application specific
//! table.
//!
//! [ServeDir::mime_fn]: super::ServeDir::mime_fn

use std::path::Path;

/// content type used when the extension is unknown, absent or not valid utf-8.
pub(super) const OCTET_STREAM: &str = "application/octet-stream";

/// longest extension in [TYPES]. anything longer can not match and skips the lowercasing.
const MAX_EXT: usize = 5;

/// extension to content type, sorted by extension for [slice::binary_search_by_key].
///
/// entries must stay ASCII: the lookup compares raw bytes, which only orders the same way
/// as [str] for ASCII. `table_is_sorted_and_within_max_ext` enforces both.
///
/// text types carry no `charset` parameter. the encoding of a file on disk is not known
/// here and a wrong charset is worse than none, which leaves the client to sniff.
const TYPES: &[(&str, &str)] = &[
    ("7z", "application/x-7z-compressed"),
    ("aac", "audio/aac"),
    ("apng", "image/apng"),
    ("atom", "application/atom+xml"),
    ("avi", "video/x-msvideo"),
    ("avif", "image/avif"),
    ("bin", OCTET_STREAM),
    ("bmp", "image/bmp"),
    ("br", "application/x-brotli"),
    ("bz2", "application/x-bzip2"),
    ("css", "text/css"),
    ("csv", "text/csv"),
    ("eot", "application/vnd.ms-fontobject"),
    ("epub", "application/epub+zip"),
    ("flac", "audio/flac"),
    ("gif", "image/gif"),
    ("gz", "application/gzip"),
    ("htm", "text/html"),
    ("html", "text/html"),
    ("ico", "image/x-icon"),
    ("ics", "text/calendar"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("jsonl", "application/jsonl"),
    ("m4a", "audio/mp4"),
    ("map", "application/json"),
    ("md", "text/markdown"),
    ("mjs", "text/javascript"),
    ("mov", "video/quicktime"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("oga", "audio/ogg"),
    ("ogg", "audio/ogg"),
    ("ogv", "video/ogg"),
    ("opus", "audio/ogg"),
    ("otf", "font/otf"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("rss", "application/rss+xml"),
    ("svg", "image/svg+xml"),
    ("tar", "application/x-tar"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("toml", "application/toml"),
    ("ttf", "font/ttf"),
    ("txt", "text/plain"),
    ("wasm", "application/wasm"),
    ("wav", "audio/wav"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("xml", "text/xml"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("zip", "application/zip"),
    ("zst", "application/zstd"),
];

/// guess a content type from the extension of `path`.
pub(super) fn from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;

    // extensions are short so they are lowercased into a stack buffer for a case
    // insensitive lookup without allocating.
    if ext.is_empty() || ext.len() > MAX_EXT {
        return None;
    }
    let mut buf = [0; MAX_EXT];
    let buf = &mut buf[..ext.len()];
    buf.copy_from_slice(ext.as_bytes());
    buf.make_ascii_lowercase();
    let ext: &[u8] = buf;

    TYPES
        .binary_search_by_key(&ext, |(ext, _)| ext.as_bytes())
        .ok()
        .map(|idx| TYPES[idx].1)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn table_is_sorted_and_within_max_ext() {
        assert!(
            TYPES.windows(2).all(|w| w[0].0 < w[1].0),
            "TYPES must be sorted by extension and free of duplicates for binary search"
        );
        assert_eq!(
            TYPES.iter().map(|(ext, _)| ext.len()).max().unwrap(),
            MAX_EXT,
            "MAX_EXT must equal the longest extension in TYPES"
        );
        assert!(
            TYPES
                .iter()
                .all(|(ext, _)| ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
        );
    }

    #[test]
    fn common_types() {
        assert_eq!(from_path(Path::new("index.html")), Some("text/html"));
        assert_eq!(from_path(Path::new("test.txt")), Some("text/plain"));
        assert_eq!(from_path(Path::new("app.js")), Some("text/javascript"));
        assert_eq!(from_path(Path::new("app.wasm")), Some("application/wasm"));
        assert_eq!(from_path(Path::new("font.woff2")), Some("font/woff2"));
        assert_eq!(from_path(Path::new("a/b/c/photo.jpeg")), Some("image/jpeg"));
    }

    #[test]
    fn extension_is_case_insensitive() {
        assert_eq!(from_path(Path::new("PHOTO.JPG")), Some("image/jpeg"));
        assert_eq!(from_path(Path::new("Index.HtMl")), Some("text/html"));
    }

    #[test]
    fn only_the_last_extension_counts() {
        assert_eq!(from_path(Path::new("archive.tar.gz")), Some("application/gzip"));
        assert_eq!(from_path(Path::new("app.min.js")), Some("text/javascript"));
    }

    #[test]
    fn unknown_extension() {
        assert_eq!(from_path(Path::new("file.dwg")), None);
        assert_eq!(from_path(Path::new("file")), None);
        assert_eq!(from_path(Path::new("file.")), None);
        // a dotfile has no extension.
        assert_eq!(from_path(Path::new(".gitignore")), None);
        // longer than any entry in the table.
        assert_eq!(from_path(Path::new("file.extension")), None);
    }
}
