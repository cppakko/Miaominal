use super::super::*;
use crate::ui::i18n;
use base64::Engine as _;
use gpui_component::WindowExt as _;
use miaominal_core::chat_attachment::{
    ChatAttachment, ChatAttachmentContent, ChatImage, ChatTextFile, MAX_ATTACHMENTS_PER_MESSAGE,
    MAX_IMAGE_DIMENSION, MAX_IMAGE_SIZE_BYTES, MAX_TEXT_FILE_SIZE_BYTES,
};
use std::path::Path;

/// Result of ingesting a single candidate file/bytes into a `ChatAttachment`.
type IngestResult = std::result::Result<ChatAttachment, AttachmentError>;

/// Discriminated attachment ingestion error so the UI can show a specific
/// i18n message instead of a generic one.
#[derive(Debug, Clone)]
pub(crate) struct AttachmentError {
    pub filename: String,
    pub kind: AttachmentErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachmentErrorKind {
    UnsupportedType,
    TooLarge,
    ReadFailed,
    ImageDecodeFailed,
    InvalidText,
}

impl AttachmentError {
    fn new(filename: String, kind: AttachmentErrorKind) -> Self {
        Self { filename, kind }
    }
}

/// Reads, validates, and builds a `ChatAttachment` from a local file path.
///
/// Detection order:
/// 1. Magic bytes in the file header — if an image signature is found the
///    file is treated as an image regardless of its extension.
/// 2. Extension-based classification — fallback when no magic bytes match.
/// 3. UTF-8 validity — files with unknown extensions that happen to be valid
///    UTF-8 are accepted as plain text.
///
/// Images are decoded, scaled to at most 2048px on the longest edge, and
/// re-encoded (JPEG 85% for opaque images, PNG when an alpha channel is
/// present). Text files are read as UTF-8 and validated against the 512KB
/// limit.
pub(crate) fn build_attachment_from_path(path: &Path) -> IngestResult {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_string();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    let metadata = std::fs::metadata(path)
        .map_err(|_| AttachmentError::new(filename.clone(), AttachmentErrorKind::ReadFailed))?;
    let disk_size = metadata.len();
    if disk_size > MAX_IMAGE_SIZE_BYTES {
        return Err(AttachmentError::new(
            filename,
            AttachmentErrorKind::TooLarge,
        ));
    }

    let bytes = std::fs::read(path)
        .map_err(|_| AttachmentError::new(filename.clone(), AttachmentErrorKind::ReadFailed))?;

    if let Some(detected_ext) = detect_image_format_from_bytes(&bytes) {
        return build_image_attachment(&filename, detected_ext, &bytes);
    }

    let is_image_ext = miaominal_core::chat_attachment::is_image_extension(&extension);
    if is_image_ext {
        return build_image_attachment(&filename, &extension, &bytes);
    }

    let is_text_ext = miaominal_core::chat_attachment::is_text_extension(&extension);
    if is_text_ext {
        if disk_size > MAX_TEXT_FILE_SIZE_BYTES {
            return Err(AttachmentError::new(
                filename,
                AttachmentErrorKind::TooLarge,
            ));
        }
        return build_text_attachment(&filename, &extension, &bytes);
    }

    if String::from_utf8(bytes.to_vec()).is_ok() {
        if disk_size > MAX_TEXT_FILE_SIZE_BYTES {
            return Err(AttachmentError::new(
                filename,
                AttachmentErrorKind::TooLarge,
            ));
        }
        return build_text_attachment(&filename, &extension, &bytes);
    }

    Err(AttachmentError::new(
        filename,
        AttachmentErrorKind::UnsupportedType,
    ))
}

/// Builds a `ChatAttachment` from raw image bytes (e.g. clipboard image data).
/// The `format_hint` is a lowercase extension ("png", "jpeg", ...) used as a
/// fallback decoder when content-based auto-detection fails.
pub(crate) fn build_image_attachment(
    filename: &str,
    extension: &str,
    bytes: &[u8],
) -> IngestResult {
    let decoded = image::load_from_memory(bytes)
        .or_else(|err| {
            let format = match extension {
                "png" => image::ImageFormat::Png,
                "jpg" | "jpeg" => image::ImageFormat::Jpeg,
                "gif" => image::ImageFormat::Gif,
                "webp" => image::ImageFormat::WebP,
                "bmp" => image::ImageFormat::Bmp,
                _ => return Err(err),
            };
            image::load_from_memory_with_format(bytes, format)
        })
        .map_err(|_| {
            AttachmentError::new(filename.to_string(), AttachmentErrorKind::ImageDecodeFailed)
        })?;
    let scaled = scale_image(decoded, MAX_IMAGE_DIMENSION);
    let (mime_type, encoded) = encode_scaled_image(&scaled);
    let thumbnail = make_thumbnail(&scaled, 64);
    let thumbnail_b64 = base64::engine::general_purpose::STANDARD.encode(&thumbnail);
    let data_base64 = base64::engine::general_purpose::STANDARD.encode(&encoded);
    let width = scaled.width();
    let height = scaled.height();
    let size_bytes = encoded.len() as u64;
    Ok(ChatAttachment {
        id: uuid::Uuid::new_v4().to_string(),
        filename: filename.to_string(),
        mime_type: mime_type.clone(),
        size_bytes,
        content: ChatAttachmentContent::Image(ChatImage {
            mime_type,
            data_base64,
            thumbnail_base64: thumbnail_b64,
            width,
            height,
        }),
    })
}

/// Builds a `ChatAttachment` from raw text file bytes. The bytes must be valid
/// UTF-8; otherwise an error is returned.
pub(crate) fn build_text_attachment(filename: &str, extension: &str, bytes: &[u8]) -> IngestResult {
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
        AttachmentError::new(filename.to_string(), AttachmentErrorKind::InvalidText)
    })?;
    let language = miaominal_core::chat_attachment::extension_to_language(extension);
    let mime_type = miaominal_core::chat_attachment::extension_to_mime(extension);
    let size_bytes = text.len() as u64;
    Ok(ChatAttachment {
        id: uuid::Uuid::new_v4().to_string(),
        filename: filename.to_string(),
        mime_type,
        size_bytes,
        content: ChatAttachmentContent::TextFile(ChatTextFile { text, language }),
    })
}

/// Maps a GPUI `ImageFormat` to the lowercase extension used by the decoder.
pub(crate) fn gpui_image_format_to_extension(format: gpui::ImageFormat) -> &'static str {
    match format {
        gpui::ImageFormat::Png => "png",
        gpui::ImageFormat::Jpeg => "jpeg",
        gpui::ImageFormat::Webp => "webp",
        gpui::ImageFormat::Gif => "gif",
        gpui::ImageFormat::Bmp => "bmp",
        _ => "png",
    }
}

/// Scales an image so its longest edge is at most `max_dimension` pixels.
/// Smaller images are returned unchanged.
fn scale_image(image: image::DynamicImage, max_dimension: u32) -> image::DynamicImage {
    let (width, height) = (image.width(), image.height());
    let longest = width.max(height);
    if longest <= max_dimension {
        return image;
    }
    let scale = max_dimension as f32 / longest as f32;
    let new_width = (width as f32 * scale).round() as u32;
    let new_height = (height as f32 * scale).round() as u32;
    image.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
}

/// Detects supported image formats from file-header magic bytes.
fn detect_image_format_from_bytes(bytes: &[u8]) -> Option<&'static str> {
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

/// Encodes the scaled image for storage. Opaque images use JPEG at 85%
/// quality; images with an alpha channel use PNG to preserve transparency.
fn encode_scaled_image(image: &image::DynamicImage) -> (String, Vec<u8>) {
    let has_alpha = image.color().has_alpha();
    if has_alpha {
        let mut buffer = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
        if let Err(error) = image.write_with_encoder(encoder) {
            log::warn!("failed to encode PNG attachment: {error:?}");
        }
        ("image/png".to_string(), buffer)
    } else {
        let mut buffer = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 85);
        if let Err(error) = image.write_with_encoder(encoder) {
            log::warn!("failed to encode JPEG attachment: {error:?}");
        }
        ("image/jpeg".to_string(), buffer)
    }
}

/// Produces a small PNG thumbnail of at most `size` pixels on the longest edge.
fn make_thumbnail(image: &image::DynamicImage, size: u32) -> Vec<u8> {
    let thumb = image.resize(size, size, image::imageops::FilterType::Nearest);
    let mut buffer = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
    if let Err(error) = thumb.write_with_encoder(encoder) {
        log::warn!("failed to encode attachment thumbnail: {error:?}");
    }
    buffer
}

impl AppView {
    /// Adds already-built attachments to the pending list, enforcing the
    /// per-message maximum. Returns the number of attachments that were
    /// accepted; surplus attachments are dropped with a status message.
    pub(in crate::ui::shell) fn add_pending_attachments(
        &mut self,
        attachments: Vec<ChatAttachment>,
        cx: &mut Context<Self>,
    ) {
        let current = self.session_agent.pending_attachments.len();
        let remaining = MAX_ATTACHMENTS_PER_MESSAGE.saturating_sub(current);
        if remaining == 0 {
            self.status_message = i18n::string_args(
                "workspace.panel.agent.messages.too_many_attachments",
                &[("limit", &MAX_ATTACHMENTS_PER_MESSAGE.to_string())],
            );
            cx.notify();
            return;
        }
        let accepted = attachments.len().min(remaining);
        let surplus = attachments.len() > accepted;
        self.session_agent
            .pending_attachments
            .extend(attachments.into_iter().take(accepted));
        if surplus {
            self.status_message = i18n::string_args(
                "workspace.panel.agent.messages.too_many_attachments",
                &[("limit", &MAX_ATTACHMENTS_PER_MESSAGE.to_string())],
            );
        } else {
            self.status_message = i18n::string_args(
                "workspace.panel.agent.messages.attachments_added",
                &[("count", &accepted.to_string())],
            );
        }
        cx.notify();
    }

    /// Removes a pending attachment by id.
    pub(in crate::ui::shell) fn remove_pending_attachment(
        &mut self,
        attachment_id: &str,
        cx: &mut Context<Self>,
    ) {
        let before = self.session_agent.pending_attachments.len();
        self.session_agent
            .pending_attachments
            .retain(|attachment| attachment.id != attachment_id);
        if self.session_agent.pending_attachments.len() != before {
            self.status_message = i18n::string("workspace.panel.agent.messages.attachment_removed");
            cx.notify();
        }
    }

    /// Opens the native file picker and ingests the selected image/text files.
    /// Disabled while a chat response is streaming.
    pub(in crate::ui::shell) fn open_attachment_picker(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.is_busy() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let files = rfd::FileDialog::new()
                .add_filter("All files", &["*"])
                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                .add_filter(
                    "Text files",
                    &[
                        "txt", "md", "rs", "py", "js", "ts", "jsx", "tsx", "json", "yaml", "yml",
                        "toml", "xml", "html", "css", "c", "cpp", "h", "hpp", "go", "java", "sh",
                        "bash", "sql", "csv", "log", "ini", "cfg", "env", "diff", "patch",
                    ],
                )
                .pick_files();
            let files = match files {
                Some(paths) if !paths.is_empty() => paths,
                _ => return,
            };
            this.update(cx, |this, cx| {
                this.ingest_attachment_paths(files, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Ingests a list of local file paths into pending attachments, reporting
    /// per-file errors via a notification toast and the status message.
    pub(in crate::ui::shell) fn ingest_attachment_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.is_busy() {
            return;
        }
        let mut attachments = Vec::new();
        let mut errors: Vec<AttachmentError> = Vec::new();
        for path in &paths {
            match build_attachment_from_path(path) {
                Ok(attachment) => attachments.push(attachment),
                Err(error) => errors.push(error),
            }
        }
        if let Some(error) = errors.first() {
            let message = attachment_error_status_message(error);
            self.status_message = message.clone();
            self.notify_attachment_error(message, cx);
        }
        if attachments.is_empty() {
            return;
        }
        self.add_pending_attachments(attachments, cx);
    }

    /// Ingests raw image bytes from the clipboard (format hint is a GPUI
    /// `ImageFormat`). Used by the Ctrl+V paste handler.
    pub(in crate::ui::shell) fn ingest_clipboard_image(
        &mut self,
        format: gpui::ImageFormat,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.is_busy() {
            return;
        }
        let extension = gpui_image_format_to_extension(format);
        let filename = format!("pasted.{extension}");
        match build_image_attachment(&filename, extension, &bytes) {
            Ok(attachment) => {
                self.add_pending_attachments(vec![attachment], cx);
            }
            Err(error) => {
                let message = attachment_error_status_message(&error);
                self.status_message = message.clone();
                self.notify_attachment_error(message, cx);
            }
        }
    }

    /// Pushes a notification toast for an attachment ingestion error.
    fn notify_attachment_error(&mut self, message: String, cx: &mut Context<Self>) {
        let title = i18n::string("notifications.attachment.title");
        let notification = Self::error_notification(title, message);
        self.with_active_window(cx, move |window, cx| {
            window.push_notification(notification, cx);
        });
    }
}

/// Maps an `AttachmentError` to a localised status message string.
fn attachment_error_status_message(error: &AttachmentError) -> String {
    match error.kind {
        AttachmentErrorKind::UnsupportedType => i18n::string_args(
            "workspace.panel.agent.messages.unsupported_file_type",
            &[("filename", &error.filename)],
        ),
        AttachmentErrorKind::TooLarge => i18n::string_args(
            "workspace.panel.agent.messages.file_too_large",
            &[("filename", &error.filename)],
        ),
        AttachmentErrorKind::ReadFailed => i18n::string_args(
            "workspace.panel.agent.messages.attachment_load_failed",
            &[("filename", &error.filename)],
        ),
        AttachmentErrorKind::ImageDecodeFailed => i18n::string_args(
            "workspace.panel.agent.messages.image_decode_failed",
            &[("filename", &error.filename)],
        ),
        AttachmentErrorKind::InvalidText => i18n::string_args(
            "workspace.panel.agent.messages.invalid_text_file",
            &[("filename", &error.filename)],
        ),
    }
}
