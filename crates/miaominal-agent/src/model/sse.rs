//! Incremental server-sent events decoder.
//!
//! The decoder buffers raw bytes (not strings), so network chunk boundaries may
//! fall anywhere, including inside a UTF-8 multibyte sequence or between the
//! `\r` and `\n` of a line terminator. Lines end with `\n`; a trailing `\r` is
//! stripped so `\r\n` streams are handled too.

/// One dispatched SSE event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// Value of the last `event:` field, if any.
    pub event: Option<String>,
    /// All `data:` lines joined with `\n`.
    pub data: String,
}

#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
    /// Bytes of `buffer` already scanned without finding `\n`.
    scanned: usize,
    event: Option<String>,
    data: String,
    has_data: bool,
}

impl SseDecoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes and return every event completed by them.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut start = 0;
        let mut search_from = self.scanned;
        while let Some(offset) = self.buffer[search_from..].iter().position(|b| *b == b'\n') {
            let end = search_from + offset;
            let mut line_end = end;
            if line_end > start && self.buffer[line_end - 1] == b'\r' {
                line_end -= 1;
            }
            let line = String::from_utf8_lossy(&self.buffer[start..line_end]).into_owned();
            if let Some(event) = self.process_line(&line) {
                events.push(event);
            }
            start = end + 1;
            search_from = start;
        }
        self.buffer.drain(..start);
        self.scanned = self.buffer.len();
        events
    }

    /// Flush at end of input: a final unterminated line is processed and any
    /// pending event is dispatched.
    pub(crate) fn finish(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let mut bytes = std::mem::take(&mut self.buffer);
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            let line = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(event) = self.process_line(&line) {
                events.push(event);
            }
        }
        self.scanned = 0;
        if let Some(event) = self.dispatch() {
            events.push(event);
        }
        events
    }

    fn process_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_owned()),
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        if !self.has_data {
            return None;
        }
        self.has_data = false;
        Some(SseEvent {
            event,
            data: std::mem::take(&mut self.data),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    fn decode_in_chunks(input: &[u8], chunk: usize) -> Vec<SseEvent> {
        let mut decoder = SseDecoder::new();
        let mut events = Vec::new();
        for piece in input.chunks(chunk) {
            events.extend(decoder.push(piece));
        }
        events.extend(decoder.finish());
        events
    }

    #[test]
    fn decodes_lf_and_crlf_streams() {
        let lf = b"data: one\n\nevent: ping\ndata: {}\n\n";
        let crlf = b"data: one\r\n\r\nevent: ping\r\ndata: {}\r\n\r\n";
        let expected = vec![ev(None, "one"), ev(Some("ping"), "{}")];
        assert_eq!(decode_in_chunks(lf, 1024), expected);
        assert_eq!(decode_in_chunks(crlf, 1024), expected);
    }

    #[test]
    fn joins_multi_line_data_and_skips_comments() {
        let input = b": keep-alive\ndata: line1\ndata:line2\ndata\n\n: OPENROUTER PROCESSING\n\n";
        assert_eq!(
            decode_in_chunks(input, 1024),
            vec![ev(None, "line1\nline2\n")]
        );
    }

    #[test]
    fn every_chunk_boundary_yields_same_events() {
        let input =
            "event: content_block_delta\r\ndata: {\"text\":\"你好🐱\"}\r\n\r\ndata: [DONE]\n\n"
                .as_bytes();
        let expected = vec![
            ev(Some("content_block_delta"), "{\"text\":\"你好🐱\"}"),
            ev(None, "[DONE]"),
        ];
        for size in 1..input.len() {
            assert_eq!(decode_in_chunks(input, size), expected, "chunk size {size}");
        }
        // Split exactly inside the 4-byte emoji.
        let emoji_at = input.windows(4).position(|w| w == "🐱".as_bytes()).unwrap();
        let mut decoder = SseDecoder::new();
        let mut events = decoder.push(&input[..emoji_at + 2]);
        events.extend(decoder.push(&input[emoji_at + 2..]));
        assert_eq!(events, expected);
    }

    #[test]
    fn split_between_cr_and_lf_does_not_dispatch_early() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(b"data: a\r").is_empty());
        assert!(decoder.push(b"\n\r").is_empty());
        assert_eq!(decoder.push(b"\n"), vec![ev(None, "a")]);
    }

    #[test]
    fn finish_flushes_unterminated_event() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(b"data: tail").is_empty());
        assert_eq!(decoder.finish(), vec![ev(None, "tail")]);
        assert!(decoder.finish().is_empty());
    }

    #[test]
    fn event_without_data_is_not_dispatched() {
        assert!(decode_in_chunks(b"event: ping\n\n", 3).is_empty());
    }
}
