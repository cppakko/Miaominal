use super::{AgentController, AppCommand};
use crate::ui::{i18n, shell::error_notification};
use base64::Engine as _;
use gpui::{Context, ImageFormat};
use gpui_component::WindowExt as _;
use miaominal_core::chat_attachment::{
    ChatAttachment, ChatAttachmentContent, ChatImage, ChatTextFile, MAX_ATTACHMENTS_PER_MESSAGE,
    MAX_IMAGE_DIMENSION, MAX_IMAGE_SIZE_BYTES, MAX_TEXT_FILE_SIZE_BYTES,
};
use std::path::{Path, PathBuf};

type IngestResult = std::result::Result<ChatAttachment, AttachmentError>;

#[derive(Debug, Default)]
struct AttachmentIngestOutcome {
    status: Option<String>,
    error_notification: Option<String>,
}

#[derive(Debug, Clone)]
struct AttachmentError {
    filename: String,
    kind: AttachmentErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachmentErrorKind {
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

impl AgentController {
    pub(in crate::ui::shell) fn open_attachment_picker(&self, cx: &mut Context<Self>) {
        if self.session_agent().is_busy() {
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
            let _ = this.update(cx, |controller, cx| {
                controller.ingest_attachment_paths_and_report(files, cx);
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn ingest_attachment_paths_and_report(
        &mut self,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.ingest_attachment_paths(paths, cx);
        self.apply_attachment_ingest_outcome(outcome, cx);
    }

    pub(in crate::ui::shell) fn ingest_clipboard_image_and_report(
        &mut self,
        format: ImageFormat,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.ingest_clipboard_image(format, bytes, cx);
        self.apply_attachment_ingest_outcome(outcome, cx);
    }

    pub(in crate::ui::shell) fn remove_pending_attachment_and_report(
        &mut self,
        attachment_id: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(status) = self.remove_pending_attachment(attachment_id, cx) {
            cx.emit(AppCommand::Feedback(status));
            cx.notify();
        }
    }

    fn apply_attachment_ingest_outcome(
        &self,
        outcome: AttachmentIngestOutcome,
        cx: &mut Context<Self>,
    ) {
        let changed = outcome.status.is_some() || outcome.error_notification.is_some();
        if let Some(status) = outcome.status {
            cx.emit(AppCommand::Feedback(status));
        }
        if let Some(message) = outcome.error_notification {
            let title = i18n::string("notifications.attachment.title");
            let notification = error_notification(title, message);
            if let Some(window_handle) = cx.active_window()
                && let Err(error) = window_handle.update(cx, move |_, window, cx| {
                    window.push_notification(notification, cx);
                })
            {
                log::debug!("failed to show attachment notification: {error:?}");
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn ingest_attachment_paths(
        &mut self,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) -> AttachmentIngestOutcome {
        if self.runtime.get_mut().foreground.is_busy() {
            return AttachmentIngestOutcome::default();
        }

        let mut attachments = Vec::new();
        let mut first_error = None;
        for path in &paths {
            match build_attachment_from_path(path) {
                Ok(attachment) => attachments.push(attachment),
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }

        let error_notification = first_error.as_ref().map(attachment_error_status_message);
        if attachments.is_empty() {
            return AttachmentIngestOutcome {
                status: error_notification.clone(),
                error_notification,
            };
        }

        let status = self.add_pending_attachments(attachments);
        cx.notify();
        AttachmentIngestOutcome {
            status,
            error_notification,
        }
    }

    fn ingest_clipboard_image(
        &mut self,
        format: gpui::ImageFormat,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) -> AttachmentIngestOutcome {
        if self.runtime.get_mut().foreground.is_busy() {
            return AttachmentIngestOutcome::default();
        }

        let extension = gpui_image_format_to_extension(format);
        let filename = format!("pasted.{extension}");
        match build_image_attachment(&filename, extension, &bytes) {
            Ok(attachment) => {
                let status = self.add_pending_attachments(vec![attachment]);
                cx.notify();
                AttachmentIngestOutcome {
                    status,
                    error_notification: None,
                }
            }
            Err(error) => {
                let message = attachment_error_status_message(&error);
                AttachmentIngestOutcome {
                    status: Some(message.clone()),
                    error_notification: Some(message),
                }
            }
        }
    }

    fn remove_pending_attachment(
        &mut self,
        attachment_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let state = &mut self.runtime.get_mut().foreground;
        let before = state.pending_attachments.len();
        state
            .pending_attachments
            .retain(|attachment| attachment.id != attachment_id);
        if state.pending_attachments.len() == before {
            return None;
        }

        cx.notify();
        Some(i18n::string(
            "workspace.panel.agent.messages.attachment_removed",
        ))
    }

    fn add_pending_attachments(&mut self, attachments: Vec<ChatAttachment>) -> Option<String> {
        if attachments.is_empty() {
            return None;
        }

        let state = &mut self.runtime.get_mut().foreground;
        let remaining = MAX_ATTACHMENTS_PER_MESSAGE.saturating_sub(state.pending_attachments.len());
        if remaining == 0 {
            return Some(too_many_attachments_message());
        }

        let accepted = attachments.len().min(remaining);
        let surplus = attachments.len() > accepted;
        state
            .pending_attachments
            .extend(attachments.into_iter().take(accepted));
        Some(if surplus {
            too_many_attachments_message()
        } else {
            i18n::string_args(
                "workspace.panel.agent.messages.attachments_added",
                &[("count", &accepted.to_string())],
            )
        })
    }
}

fn too_many_attachments_message() -> String {
    i18n::string_args(
        "workspace.panel.agent.messages.too_many_attachments",
        &[("limit", &MAX_ATTACHMENTS_PER_MESSAGE.to_string())],
    )
}

fn build_attachment_from_path(path: &Path) -> IngestResult {
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
    if miaominal_core::chat_attachment::is_image_extension(&extension) {
        return build_image_attachment(&filename, &extension, &bytes);
    }
    if miaominal_core::chat_attachment::is_text_extension(&extension) {
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

fn build_image_attachment(filename: &str, extension: &str, bytes: &[u8]) -> IngestResult {
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

fn build_text_attachment(filename: &str, extension: &str, bytes: &[u8]) -> IngestResult {
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

fn gpui_image_format_to_extension(format: gpui::ImageFormat) -> &'static str {
    match format {
        gpui::ImageFormat::Png => "png",
        gpui::ImageFormat::Jpeg => "jpeg",
        gpui::ImageFormat::Webp => "webp",
        gpui::ImageFormat::Gif => "gif",
        gpui::ImageFormat::Bmp => "bmp",
        _ => "png",
    }
}

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

fn make_thumbnail(image: &image::DynamicImage, size: u32) -> Vec<u8> {
    let thumb = image.resize(size, size, image::imageops::FilterType::Nearest);
    let mut buffer = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
    if let Err(error) = thumb.write_with_encoder(encoder) {
        log::warn!("failed to encode attachment thumbnail: {error:?}");
    }
    buffer
}

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
