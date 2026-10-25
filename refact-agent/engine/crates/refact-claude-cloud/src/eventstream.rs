use base64::Engine;
use std::collections::HashMap;

const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub struct BedrockEventStreamDecoder {
    buffer: Vec<u8>,
    failure: Option<String>,
}

impl BedrockEventStreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, mut bytes: &[u8]) -> Result<Vec<String>, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let mut output = Vec::new();
        match self.decode(&mut bytes, &mut output) {
            Ok(()) => Ok(output),
            Err(error) => {
                self.failure = Some(error.clone());
                self.buffer.clear();
                if output.is_empty() {
                    Err(error)
                } else {
                    Ok(output)
                }
            }
        }
    }

    fn decode(&mut self, bytes: &mut &[u8], output: &mut Vec<String>) -> Result<(), String> {
        loop {
            if self.buffer.len() < 12 {
                let take = (12 - self.buffer.len()).min(bytes.len());
                self.buffer.extend_from_slice(&bytes[..take]);
                *bytes = &bytes[take..];
                if self.buffer.len() < 12 {
                    break;
                }
            }
            let total = u32_at(&self.buffer, 0) as usize;
            let headers = u32_at(&self.buffer, 4) as usize;
            if !(16..=MAX_FRAME).contains(&total) || headers > total - 16 {
                return Err("Invalid Bedrock frame lengths or size bound".into());
            }
            if crc32fast::hash(&self.buffer[..8]) != u32_at(&self.buffer, 8) {
                return Err("Bedrock prelude CRC mismatch".into());
            }
            let take = (total - self.buffer.len()).min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            *bytes = &bytes[take..];
            if self.buffer.len() < total {
                break;
            }
            if crc32fast::hash(&self.buffer[..total - 4]) != u32_at(&self.buffer, total - 4) {
                return Err("Bedrock message CRC mismatch".into());
            }
            let metadata = parse_headers(&self.buffer[12..12 + headers])?;
            let payload = &self.buffer[12 + headers..total - 4];
            match metadata.get(":message-type").map(String::as_str) {
                Some("exception" | "error") => return Err(exception_message(&metadata, payload)),
                Some("event")
                    if metadata.get(":event-type").map(String::as_str) == Some("chunk") =>
                {
                    let wrapper: serde_json::Value = serde_json::from_slice(payload)
                        .map_err(|_| "Invalid Bedrock chunk JSON")?;
                    let encoded = wrapper
                        .get("bytes")
                        .and_then(|v| v.as_str())
                        .ok_or("Bedrock chunk missing bytes")?;
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|_| "Invalid Bedrock chunk base64")?;
                    let text =
                        String::from_utf8(decoded).map_err(|_| "Invalid Bedrock chunk UTF-8")?;
                    let _: serde_json::Value =
                        serde_json::from_str(&text).map_err(|_| "Invalid Bedrock event JSON")?;
                    output.push(text);
                }
                _ => return Err("Unexpected Bedrock event metadata".into()),
            }
            self.buffer.clear();
            if bytes.is_empty() {
                break;
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if !self.buffer.is_empty() {
            self.failure = Some("Truncated Bedrock event stream".into());
            return Err("Truncated Bedrock event stream".into());
        }
        Ok(())
    }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn exception_message(metadata: &HashMap<String, String>, _payload: &[u8]) -> String {
    let kind = metadata
        .get(":exception-type")
        .or_else(|| metadata.get(":error-code"))
        .map(String::as_str)
        .unwrap_or("unknown");
    let (kind, message) = match kind {
        "internalServerException" => ("internalServerException", "internal server error"),
        "modelStreamErrorException" => ("modelStreamErrorException", "stream error"),
        "modelTimeoutException" => ("modelTimeoutException", "request timed out"),
        "throttlingException" => ("throttlingException", "rate limit exceeded"),
        "serviceUnavailableException" => ("serviceUnavailableException", "service unavailable"),
        "validationException" => ("validationException", "invalid request"),
        "accessDeniedException" => ("accessDeniedException", "access denied"),
        "resourceNotFoundException" => ("resourceNotFoundException", "resource not found"),
        _ => ("unknown", "provider rejected stream"),
    };
    format!("Bedrock stream exception: {kind}: {message}")
}

fn parse_headers(mut bytes: &[u8]) -> Result<HashMap<String, String>, String> {
    let mut headers = HashMap::new();
    while !bytes.is_empty() {
        let len = bytes[0] as usize;
        if bytes.len() < len + 2 {
            return Err("Truncated Bedrock header".into());
        }
        let name = std::str::from_utf8(&bytes[1..1 + len])
            .map_err(|_| "Invalid Bedrock header name")?
            .to_string();
        let kind = bytes[1 + len];
        bytes = &bytes[len + 2..];
        let (size, prefix) = match kind {
            0 | 1 => (0, 0),
            2 => (1, 0),
            3 => (2, 0),
            4 => (4, 0),
            5 | 8 => (8, 0),
            9 => (16, 0),
            6 | 7 => {
                if bytes.len() < 2 {
                    return Err("Truncated Bedrock header length".into());
                }
                (u16::from_be_bytes([bytes[0], bytes[1]]) as usize, 2)
            }
            _ => return Err("Unknown Bedrock header type".into()),
        };
        if bytes.len() < size + prefix {
            return Err("Truncated Bedrock header value".into());
        }
        if name.starts_with(':') {
            if kind != 7 {
                return Err("Bedrock metadata must be strings".into());
            }
            let value = std::str::from_utf8(&bytes[prefix..prefix + size])
                .map_err(|_| "Invalid Bedrock header UTF-8")?
                .to_string();
            if headers.insert(name, value).is_some() {
                return Err("Duplicate Bedrock metadata".into());
            }
        }
        bytes = &bytes[prefix + size..];
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(kind: &str, event: &str, payload: &[u8]) -> Vec<u8> {
        let mut headers = Vec::new();
        let event_header = if kind == "exception" {
            ":exception-type"
        } else if kind == "error" {
            ":error-code"
        } else {
            ":event-type"
        };
        for (k, v) in [(":message-type", kind), (event_header, event)] {
            headers.push(k.len() as u8);
            headers.extend(k.as_bytes());
            headers.push(7);
            headers.extend((v.len() as u16).to_be_bytes());
            headers.extend(v.as_bytes());
        }
        let mut bytes = Vec::new();
        bytes.extend(((16 + headers.len() + payload.len()) as u32).to_be_bytes());
        bytes.extend((headers.len() as u32).to_be_bytes());
        bytes.extend(crc32fast::hash(&bytes).to_be_bytes());
        bytes.extend(headers);
        bytes.extend(payload);
        bytes.extend(crc32fast::hash(&bytes).to_be_bytes());
        bytes
    }
    fn chunk() -> Vec<u8> {
        frame(
            "event",
            "chunk",
            br#"{"bytes":"eyJ0eXBlIjoibWVzc2FnZV9zdG9wIn0="}"#,
        )
    }
    #[test]
    fn fragmented_multiple_and_finish() {
        let bytes = [chunk(), chunk()].concat();
        for split in 0..=bytes.len() {
            let mut d = BedrockEventStreamDecoder::new();
            let mut result = d.push(&bytes[..split]).unwrap();
            result.extend(d.push(&bytes[split..]).unwrap());
            d.finish().unwrap();
            assert_eq!(
                result,
                vec![r#"{"type":"message_stop"}"#, r#"{"type":"message_stop"}"#]
            );
        }
        let mut d = BedrockEventStreamDecoder::new();
        let mut result = Vec::new();
        for b in &bytes {
            result.extend(d.push(&[*b]).unwrap());
        }
        assert_eq!(result.len(), 2);
        d.finish().unwrap();
    }
    #[test]
    fn crc_lengths_bounds_and_truncation() {
        for index in [0, 8, 20] {
            let mut bytes = chunk();
            bytes[index] ^= 1;
            assert!(BedrockEventStreamDecoder::new().push(&bytes).is_err());
        }
        for len in [0, 15, (MAX_FRAME + 1) as u32] {
            let mut bytes = vec![];
            bytes.extend(len.to_be_bytes());
            bytes.extend(0u32.to_be_bytes());
            bytes.extend(crc32fast::hash(&bytes).to_be_bytes());
            assert!(BedrockEventStreamDecoder::new().push(&bytes).is_err());
        }
        for len in 1..chunk().len() {
            let mut d = BedrockEventStreamDecoder::new();
            d.push(&chunk()[..len]).unwrap();
            assert!(d.finish().is_err());
        }
    }
    #[test]
    fn valid_events_survive_later_exception() {
        let bytes = [
            chunk(),
            frame(
                "exception",
                "internalServerException",
                br#"{"message":"secret-provider-detail"}"#,
            ),
        ]
        .concat();
        let mut decoder = BedrockEventStreamDecoder::new();
        assert_eq!(
            decoder.push(&bytes).unwrap(),
            vec![r#"{"type":"message_stop"}"#]
        );
        let error = decoder.finish().unwrap_err();
        assert_eq!(
            error,
            "Bedrock stream exception: internalServerException: internal server error"
        );
        assert_eq!(decoder.push(&[]).unwrap_err(), error);
        assert!(!error.contains("secret-provider-detail"));
    }
    #[test]
    fn exception_classes_are_bounded_and_do_not_echo_payloads() {
        for (kind, message) in [
            ("modelStreamErrorException", "stream error"),
            ("throttlingException", "rate limit exceeded"),
            ("validationException", "invalid request"),
        ] {
            let bytes = frame("error", kind, br#"{"message":"Bearer credential-secret"}"#);
            let error = BedrockEventStreamDecoder::new().push(&bytes).unwrap_err();
            assert!(error.ends_with(message));
            assert!(!error.contains("credential-secret"));
            assert!(error.len() < 160);
        }
        let bytes = frame("exception", &"sensitive".repeat(1000), b"{}");
        assert!(
            BedrockEventStreamDecoder::new()
                .push(&bytes)
                .unwrap_err()
                .len()
                < 160
        );
    }
    #[test]
    fn strict_payload_and_exception_errors() {
        for bytes in [
            frame("exception", "internalServerException", b"{}"),
            frame("event", "unknown", b"{}"),
            frame("event", "chunk", b"{}"),
            frame("event", "chunk", br#"{"bytes":"!"}"#),
            frame("event", "chunk", br#"{"bytes":"eA=="}"#),
        ] {
            let mut d = BedrockEventStreamDecoder::new();
            assert!(d.push(&bytes).is_err());
            assert!(d.finish().is_err());
        }
    }
}
