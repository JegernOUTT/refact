/// A single SSE dispatch. Empty data is meaningful; comments/ID-only frames are not events.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Frame {
    pub data: Option<String>,
    pub id: Option<String>,
    pub event: Option<String>,
    pub retry_ms: Option<u64>,
}
pub(super) fn parse(raw: &str) -> Frame {
    let mut frame = Frame::default();
    let mut lines = Vec::new();
    for line in raw.lines() {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => lines.push(value),
            "id" if !value.contains('\0') => frame.id = Some(value.into()),
            "event" => frame.event = Some(value.into()),
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                frame.retry_ms = value.parse().ok()
            }
            _ => {}
        }
    }
    if !lines.is_empty() {
        frame.data = Some(lines.join("\n"));
    }
    frame
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fields_and_empty_data_are_not_lost() {
        let f = parse(":heartbeat\r\nid: stable42\r\nevent: update\r\nretry: 1200\r\ndata: one\r\ndata: two\r\n");
        assert_eq!(f.id.as_deref(), Some("stable42"));
        assert_eq!(f.event.as_deref(), Some("update"));
        assert_eq!(f.retry_ms, Some(1200));
        assert_eq!(f.data.as_deref(), Some("one\ntwo"));
        assert_eq!(parse("data:\n").data.as_deref(), Some(""));
        assert_eq!(parse("id: no-dispatch\n").data, None);
        assert_eq!(parse("id: bad\0id\nretry: -1\n").id, None);
    }
}
