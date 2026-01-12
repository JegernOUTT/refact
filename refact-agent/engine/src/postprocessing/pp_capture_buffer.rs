use std::collections::VecDeque;

fn truncate_to_byte_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[derive(Clone, Copy, Debug, Default)]
pub enum KeepStrategy {
    #[default]
    Head,
    HeadAndTail,
}

pub struct CaptureBuffer {
    max_bytes: usize,
    strategy: KeepStrategy,
    head: Vec<String>,
    tail: VecDeque<String>,
    head_bytes: usize,
    tail_bytes: usize,
    total_lines: usize,
    truncated: bool,
}

impl CaptureBuffer {
    pub fn new(max_bytes: usize, strategy: KeepStrategy) -> Self {
        Self {
            max_bytes,
            strategy,
            head: Vec::new(),
            tail: VecDeque::new(),
            head_bytes: 0,
            tail_bytes: 0,
            total_lines: 0,
            truncated: false,
        }
    }

    pub fn push_line(&mut self, line: String) {
        self.total_lines += 1;

        let line = if line.len() > self.max_bytes {
            self.truncated = true;
            truncate_to_byte_boundary(&line, self.max_bytes)
        } else {
            line
        };

        let line_bytes = line.len() + 1;

        match self.strategy {
            KeepStrategy::Head => {
                if self.head_bytes + line_bytes <= self.max_bytes {
                    self.head_bytes += line_bytes;
                    self.head.push(line);
                } else {
                    self.truncated = true;
                }
            }
            KeepStrategy::HeadAndTail => {
                let head_limit = self.max_bytes * 80 / 100;
                let tail_limit = self.max_bytes * 20 / 100;

                if self.head_bytes + line_bytes <= head_limit {
                    self.head_bytes += line_bytes;
                    self.head.push(line);
                } else {
                    self.tail.push_back(line);
                    self.tail_bytes += line_bytes;
                    while self.tail_bytes > tail_limit {
                        if let Some(removed) = self.tail.pop_front() {
                            self.tail_bytes -= removed.len() + 1;
                        }
                    }
                    self.truncated = true;
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn finish(self) -> String {
        self.build_result()
    }

    pub fn take_result(&mut self) -> String {
        let result = self.build_result();
        self.head.clear();
        self.tail.clear();
        self.head_bytes = 0;
        self.tail_bytes = 0;
        self.total_lines = 0;
        self.truncated = false;
        result
    }

    fn build_result(&self) -> String {
        let mut result = self.head.join("\n");

        if self.truncated {
            let skipped = self
                .total_lines
                .saturating_sub(self.head.len())
                .saturating_sub(self.tail.len());
            if !result.is_empty() {
                result.push('\n');
            }
            let strategy_name = match self.strategy {
                KeepStrategy::Head => "head",
                KeepStrategy::HeadAndTail => "head+tail",
            };
            let limit_kb = self.max_bytes / 1024;
            if skipped > 0 {
                result.push_str(&format!(
                    "⚠️ {} lines truncated ({}KB {}) 💡 Use output_limit:'all' to see full output",
                    skipped, limit_kb, strategy_name
                ));
            } else {
                result.push_str(&format!(
                    "⚠️ Long line(s) truncated ({}KB {}) 💡 Use output_limit:'all' to see full output",
                    limit_kb, strategy_name
                ));
            }
        }

        if !self.tail.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&self.tail.iter().cloned().collect::<Vec<_>>().join("\n"));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_head_strategy() {
        let mut buf = CaptureBuffer::new(20, KeepStrategy::Head);
        buf.push_line("line1".into());
        buf.push_line("line2".into());
        buf.push_line("line3".into());
        buf.push_line("line4".into());
        let result = buf.finish();
        assert!(result.starts_with("line1"));
        assert!(result.contains("⚠️"));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_head_and_tail_strategy() {
        let mut buf = CaptureBuffer::new(50, KeepStrategy::HeadAndTail);
        for i in 1..=10 {
            buf.push_line(format!("line{}", i));
        }
        let result = buf.finish();
        assert!(result.contains("line1"));
        assert!(result.contains("line10"));
        assert!(result.contains("⚠️"));
    }
}
