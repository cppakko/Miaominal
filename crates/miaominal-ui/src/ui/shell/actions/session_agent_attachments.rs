use super::super::*;
use crate::ui::i18n;
use base64::Engine as _;
use miaominal_core::chat_attachment::{
    self, ChatAttachment, ChatAttachmentContent, ChatImage, ChatTextFile, MAX_ATTACHMENTS_PER_MESSAGE,
    MAX_IMAGE_DIMENSION, MAX_IMAGE_SIZE_BYTES, MAX_TEXT_FILE_SIZE_BYTES,
};
use std::path::Path;

/// Result of ingesting a single candidate file/bytes into a `ChatAttachment`.
/// On error the message is a user-facing i18n key argument string.
type IngestResult = std::result::Result<ChatAttachment, String>;

/// Reads, validates, and builds a `ChatAttachment` from a local file path.
///
/// Images are decoded, scaled to at most 2048px on the longest edge, and
/// re-encoded (JPEG 85% for opaque images, PNG when an alpha channel is
/// present). Text files are read as UTF-8 and validated against the 512KB
/// limit. Unsupported extensions produce an error.
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
    let is_image = chat_attachment::is_image_extension(&extension);
    let is_text = chat_attachment::is_text_extension(&extension);
    if !is_image && !is_text {
        return Err(filename);
    }

    let metadata = std::fs::metadata(path).map_err(|_| filename.clone())?;
    let size_bytes = metadata.len();
    if is_image {
        if size_bytes > MAX_IMAGE_SIZE_BYTES {
            return Err(filename);
        }
        let bytes = std::fs::read(path).map_err(|_| filename.clone())?;
        build_image_attachment(&filename, &extension, &bytes)
    } else {
        if size_bytes > MAX_TEXT_FILE_SIZE_BYTES {
            return Err(filename);
        }
        let bytes = std::fs::read(path).map_err(|_| filename.clone())?;
        build_text_attachment(&filename, &extension, &bytes)
    }
}

/// Builds a `ChatAttachment` from raw image bytes (e.g. clipboard image data).
/// The `format_hint` is a lowercase extension ("png", "jpeg", ...) used to
/// pick the decoder; the image is scaled and re-encoded at ingestion time.
pub(crate) fn build_image_attachment(filename: &str, extension: &str, bytes: &[u8]) -> IngestResult {
    let format = match extension {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "gif" => image::ImageFormat::Gif,
        "webp" => image::ImageFormat::WebP,
        "bmp" => image::ImageFormat::Bmp,
        _ => return Err(filename.to_string()),
    };
    let decoded = image::load_from_memory_with_format(bytes, format)
        .map_err(|_| filename.to_string())?;
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
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| filename.to_string())?;
    let language = chat_attachment::extension_to_language(extension);
    let mime_type = chat_attachment::extension_to_mime(extension);
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

/// Encodes the scaled image for storage. Opaque images use JPEG at 85%
/// quality; images with an alpha channel use PNG to preserve transparency.
fn encode_scaled_image(image: &image::DynamicImage) -> (String, Vec<u8>) {
    let has_alpha = image.color().has_alpha();
    if has_alpha {
        let mut buffer = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
        let _ = image.write_with_encoder(encoder);
        ("image/png".to_string(), buffer)
    } else {
        let mut buffer = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 85);
        let _ = image.write_with_encoder(encoder);
        ("image/jpeg".to_string(), buffer)
    }
}

/// Produces a small PNG thumbnail of at most `size` pixels on the longest edge.
fn make_thumbnail(image: &image::DynamicImage, size: u32) -> Vec<u8> {
    let thumb = image.resize(size, size, image::imageops::FilterType::Nearest);
    let mut buffer = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
    let _ = thumb.write_with_encoder(encoder);
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
            self.status_message =
                i18n::string("workspace.panel.agent.messages.attachment_removed");
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
        let entity = cx.entity();
        cx.spawn(async move |this, cx| {
            let files = rfd::FileDialog::new()
                .add_filter(
                    "Images",
                    &["png", "jpg", "jpeg", "gif", "webp", "bmp"],
                )
                .add_filter(
                    "Text files",
                    &[
                        "txt", "md", "rs", "py", "js", "ts", "jsx", "tsx", "json", "yaml", "yml",
                        "toml", "xml", "html", "css", "c", "cpp", "h", "hpp", "go", "java", "sh",
                        "bash", "sql", "csv", "log", "ini", "cfg", "env", "diff", "patch",
                    ],
                )
                .add_filter("All files", &["*"])
                .pick_files();
            let files = match files {
                Some(paths) if !paths.is_empty() => paths,
                _ => return,
            };
            this.update(cx, |this, cx| {
                this.ingest_attachment_paths(files, cx);
            })
            .ok();
            let _ = entity;
        })
        .detach();
    }

    /// Ingests a list of local file paths into pending attachments, reporting
    /// per-file errors via the status message.
    pub(in crate::ui::shell) fn ingest_attachment_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.is_busy() {
            return;
        }
        let mut attachments = Vec::new();
        let mut last_error_filename: Option<String> = None;
        for path in &paths {
            match build_attachment_from_path(path) {
                Ok(attachment) => attachments.push(attachment),
                Err(filename) => last_error_filename = Some(filename),
            }
        }
        if let Some(filename) = last_error_filename
            && attachments.is_empty()
        {
            self.status_message = i18n::string_args(
                "workspace.panel.agent.messages.attachment_load_failed",
                &[("filename", &filename)],
            );
            cx.notify();
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
            Err(_) => {
                self.status_message = i18n::string_args(
                    "workspace.panel.agent.messages.image_decode_failed",
                    &[("filename", &filename)],
                );
                cx.notify();
            }
        }
    }
}
