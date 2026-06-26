use serde::{Deserialize, Serialize};

/// Maximum number of attachments allowed per message (images + text files combined).
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 5;

/// Maximum image attachment size in bytes (10 MB).
pub const MAX_IMAGE_SIZE_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum text file attachment size in bytes (512 KB).
pub const MAX_TEXT_FILE_SIZE_BYTES: u64 = 512 * 1024;

/// Maximum image dimension (width or height) before downscaling, in pixels.
pub const MAX_IMAGE_DIMENSION: u32 = 2048;

/// A single attachment on a chat message: either an image or a text file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatAttachment {
    pub id: String,
    pub filename: String,
    /// MIME type, e.g. `image/png`, `image/jpeg`, `text/plain`, `text/x-rust`.
    pub mime_type: String,
    pub size_bytes: u64,
    pub content: ChatAttachmentContent,
}

/// Discriminated attachment payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChatAttachmentContent {
    Image(ChatImage),
    TextFile(ChatTextFile),
}

impl ChatAttachment {
    /// Returns `true` when this attachment is an image.
    pub fn is_image(&self) -> bool {
        matches!(self.content, ChatAttachmentContent::Image(_))
    }

    /// Returns `true` when this attachment is a text file.
    pub fn is_text_file(&self) -> bool {
        matches!(self.content, ChatAttachmentContent::TextFile(_))
    }

    /// Borrows the image payload if this attachment is an image.
    pub fn as_image(&self) -> Option<&ChatImage> {
        match &self.content {
            ChatAttachmentContent::Image(image) => Some(image),
            _ => None,
        }
    }

    /// Borrows the text file payload if this attachment is a text file.
    pub fn as_text_file(&self) -> Option<&ChatTextFile> {
        match &self.content {
            ChatAttachmentContent::TextFile(text_file) => Some(text_file),
            _ => None,
        }
    }
}

/// Image attachment payload: base64-encoded full-resolution (post-scaling) image
/// bytes plus a small base64 thumbnail for preview rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatImage {
    /// MIME type of the encoded image data, e.g. `image/png`, `image/jpeg`.
    pub mime_type: String,
    /// Base64-encoded image bytes (already scaled/compressed at ingestion time).
    pub data_base64: String,
    /// Base64-encoded small thumbnail used for preview rows and message bubbles.
    pub thumbnail_base64: String,
    /// Pixel width of the full-resolution image.
    pub width: u32,
    /// Pixel height of the full-resolution image.
    pub height: u32,
}

/// Text file attachment payload: the decoded UTF-8 text plus an optional
/// detected source language identifier used for syntax hinting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatTextFile {
    /// Decoded UTF-8 text content of the file.
    pub text: String,
    /// Detected source language (e.g. `rust`, `python`), or `None` for plain text.
    pub language: Option<String>,
}

/// Classifies a file path as an image or text attachment based on its extension.
///
/// Returns `Some(true)` for image extensions, `Some(false)` for text extensions,
/// and `None` for unsupported extensions.
pub fn classify_extension(filename: &str) -> Option<bool> {
    let lower = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if is_image_extension(&lower) {
        Some(true)
    } else if is_text_extension(&lower) {
        Some(false)
    } else {
        None
    }
}

/// Returns `true` when `extension` (lowercase, no dot) is a supported image format.
pub fn is_image_extension(extension: &str) -> bool {
    matches!(
        extension,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    )
}

/// Returns `true` when `extension` (lowercase, no dot) is a supported text file format.
pub fn is_text_extension(extension: &str) -> bool {
    matches!(
        extension,
        "txt"
            | "md"
            | "markdown"
            | "rs"
            | "py"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "jsx"
            | "tsx"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "c"
            | "cpp"
            | "cc"
            | "cxx"
            | "h"
            | "hpp"
            | "hh"
            | "hxx"
            | "go"
            | "java"
            | "kt"
            | "sh"
            | "bash"
            | "zsh"
            | "sql"
            | "csv"
            | "tsv"
            | "log"
            | "ini"
            | "cfg"
            | "conf"
            | "env"
            | "diff"
            | "patch"
            | "rb"
            | "php"
            | "swift"
            | "scala"
            | "clj"
            | "lua"
            | "vim"
            | "dart"
            | "gradle"
            | "makefile"
            | "dockerfile"
    )
}

/// Detects image format from file header magic bytes, returning the lowercase
/// extension (e.g. `"png"`, `"jpeg"`) when the signature is recognised, or
/// `None` when the bytes do not match any supported image format.
pub fn detect_image_format_from_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("png");
    }
    if bytes.len() >= 3 && &bytes[0..3] == b"\xff\xd8\xff" {
        return Some("jpeg");
    }
    if bytes.len() >= 4 && bytes[0..4] == *b"GIF8" {
        return Some("gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    if bytes.len() >= 2 && &bytes[0..2] == b"BM" {
        return Some("bmp");
    }
    None
}

/// Maps a lowercase file extension (no dot) to a syntax language identifier
/// suitable for code-fence hints and `rig`/markdown rendering.
pub fn extension_to_language(extension: &str) -> Option<String> {
    let language = match extension {
        "txt" | "log" | "env" | "ini" | "cfg" | "conf" => return None,
        "md" | "markdown" => "markdown",
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" => "xml",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "c" => "c",
        "cpp" | "cc" | "cxx" => "cpp",
        "h" | "hpp" | "hh" | "hxx" => "cpp",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "sh" | "bash" | "zsh" => "bash",
        "sql" => "sql",
        "csv" => "csv",
        "tsv" => "tsv",
        "diff" | "patch" => "diff",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "scala" => "scala",
        "clj" => "clojure",
        "lua" => "lua",
        "vim" => "vim",
        "dart" => "dart",
        "gradle" => "gradle",
        "makefile" | "dockerfile" => extension,
        _ => return None,
    };
    Some(language.to_string())
}

/// Maps a lowercase file extension (no dot) to its MIME type.
pub fn extension_to_mime(extension: &str) -> String {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "xml" => "application/xml",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        _ => "text/plain",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_image_extensions() {
        assert_eq!(classify_extension("photo.png"), Some(true));
        assert_eq!(classify_extension("photo.JPG"), Some(true));
        assert_eq!(classify_extension("anim.gif"), Some(true));
        assert_eq!(classify_extension("pic.webp"), Some(true));
        assert_eq!(classify_extension("pic.bmp"), Some(true));
    }

    #[test]
    fn classifies_text_extensions() {
        assert_eq!(classify_extension("main.rs"), Some(false));
        assert_eq!(classify_extension("config.YAML"), Some(false));
        assert_eq!(classify_extension("notes.txt"), Some(false));
        assert_eq!(classify_extension("Dockerfile"), Some(false));
        assert_eq!(classify_extension("Makefile"), Some(false));
    }

    #[test]
    fn rejects_unsupported_extensions() {
        assert_eq!(classify_extension("movie.mp4"), None);
        assert_eq!(classify_extension("archive.zip"), None);
        assert_eq!(classify_extension("binary.exe"), None);
    }

    #[test]
    fn maps_extensions_to_languages() {
        assert_eq!(extension_to_language("rs"), Some("rust".to_string()));
        assert_eq!(extension_to_language("py"), Some("python".to_string()));
        assert_eq!(extension_to_language("ts"), Some("typescript".to_string()));
        assert_eq!(extension_to_language("txt"), None);
        assert_eq!(extension_to_language("log"), None);
        assert_eq!(extension_to_language("unknown"), None);
    }

    #[test]
    fn maps_extensions_to_mime_types() {
        assert_eq!(extension_to_mime("png"), "image/png");
        assert_eq!(extension_to_mime("jpeg"), "image/jpeg");
        assert_eq!(extension_to_mime("rs"), "text/plain");
        assert_eq!(extension_to_mime("md"), "text/markdown");
        assert_eq!(extension_to_mime("json"), "application/json");
    }

    #[test]
    fn attachment_accessors_distinguish_payload_kinds() {
        let image = ChatAttachment {
            id: "a1".into(),
            filename: "x.png".into(),
            mime_type: "image/png".into(),
            size_bytes: 100,
            content: ChatAttachmentContent::Image(ChatImage {
                mime_type: "image/png".into(),
                data_base64: "AAAA".into(),
                thumbnail_base64: "BBBB".into(),
                width: 10,
                height: 20,
            }),
        };
        assert!(image.is_image());
        assert!(!image.is_text_file());
        assert!(image.as_image().is_some());
        assert!(image.as_text_file().is_none());

        let text = ChatAttachment {
            id: "a2".into(),
            filename: "y.rs".into(),
            mime_type: "text/plain".into(),
            size_bytes: 200,
            content: ChatAttachmentContent::TextFile(ChatTextFile {
                text: "fn main() {}".into(),
                language: Some("rust".into()),
            }),
        };
        assert!(!text.is_image());
        assert!(text.is_text_file());
        assert!(text.as_image().is_none());
        assert!(text.as_text_file().is_some());
    }

    #[test]
    fn attachment_round_trips_through_serde_json() {
        let original = ChatAttachment {
            id: "serde-1".into(),
            filename: "main.rs".into(),
            mime_type: "text/plain".into(),
            size_bytes: 42,
            content: ChatAttachmentContent::TextFile(ChatTextFile {
                text: "fn main() {}".into(),
                language: Some("rust".into()),
            }),
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: ChatAttachment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn image_attachment_round_trips_through_serde_json() {
        let original = ChatAttachment {
            id: "serde-2".into(),
            filename: "screenshot.png".into(),
            mime_type: "image/png".into(),
            size_bytes: 4096,
            content: ChatAttachmentContent::Image(ChatImage {
                mime_type: "image/png".into(),
                data_base64: "iVBORw0KGgo=".into(),
                thumbnail_base64: "iVBOR=".into(),
                width: 800,
                height: 600,
            }),
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: ChatAttachment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn detect_image_format_from_magic_bytes() {
        assert_eq!(
            detect_image_format_from_bytes(b"\x89PNG\r\n\x1a\nrest"),
            Some("png")
        );
        assert_eq!(
            detect_image_format_from_bytes(b"\xff\xd8\xff\xe0\x00\x10JFIF"),
            Some("jpeg")
        );
        assert_eq!(
            detect_image_format_from_bytes(b"GIF89a...."),
            Some("gif")
        );
        assert_eq!(
            detect_image_format_from_bytes(b"RIFF....WEBP...."),
            Some("webp")
        );
        assert_eq!(
            detect_image_format_from_bytes(b"BM...."),
            Some("bmp")
        );
        assert_eq!(detect_image_format_from_bytes(b"not an image"), None);
        assert_eq!(detect_image_format_from_bytes(b""), None);
        assert_eq!(detect_image_format_from_bytes(b"\x89PN"), None);
    }

    #[test]
    fn size_limits_are_sane() {
        assert_eq!(MAX_IMAGE_SIZE_BYTES, 10 * 1024 * 1024);
        assert_eq!(MAX_TEXT_FILE_SIZE_BYTES, 512 * 1024);
        assert_eq!(MAX_IMAGE_DIMENSION, 2048);
        assert_eq!(MAX_ATTACHMENTS_PER_MESSAGE, 5);
    }
}
