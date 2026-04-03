use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;

use crate::models::SseEvent;

/// Parse an SSE stream from raw bytes into structured events.
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Feed raw bytes and extract any complete SSE events.
    pub fn feed(&mut self, chunk: &Bytes) -> Vec<Result<SseEvent>> {
        self.buffer
            .push_str(&String::from_utf8_lossy(chunk));

        let mut events = Vec::new();
        while let Some(event) = self.extract_event() {
            events.push(event);
        }
        events
    }

    fn extract_event(&mut self) -> Option<Result<SseEvent>> {
        // SSE events are separated by double newlines
        let separator = if self.buffer.contains("\n\n") {
            "\n\n"
        } else if self.buffer.contains("\r\n\r\n") {
            "\r\n\r\n"
        } else {
            return None;
        };

        let pos = self.buffer.find(separator)?;
        let raw_event: String = self.buffer.drain(..pos + separator.len()).collect();

        let mut event_type = String::new();
        let mut data_lines = Vec::new();

        for line in raw_event.lines() {
            if let Some(value) = line.strip_prefix("event: ") {
                event_type = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix(" ") {
                data_lines.push(value.to_string());
            } else if let Some(value) = line.strip_prefix("data:") {
                data_lines.push(value.to_string());
            }
        }

        if data_lines.is_empty() {
            return None;
        }

        let data = data_lines.join("\n");

        // Parse JSON data into SseEvent
        let result = serde_json::from_str::<SseEvent>(&data)
            .map_err(|e| anyhow::anyhow!("failed to parse SSE event '{}': {} ( {})", event_type, e, &data[..data.len().min(200)]));

        Some(result)
    }
}

/// Read SSE events from a reqwest byte stream.
pub async fn read_sse_stream(
    stream: impl futures_core::Stream<Item = reqwest::Result<Bytes>> + Unpin,
    mut callback: impl FnMut(SseEvent),
) -> Result<()> {
    let mut parser = SseParser::new();
    let mut stream = std::pin::pin!(stream);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        for event in parser.feed(&chunk) {
            match event {
                Ok(sse_event) => callback(sse_event),
                Err(e) => {
                    tracing::warn!("SSE parse error: {}", e);
                }
            }
        }
    }

    Ok(())
}
