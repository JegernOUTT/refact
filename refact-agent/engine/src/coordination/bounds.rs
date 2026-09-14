use std::io::{self, Write};
use serde::Serialize;

pub(crate) fn serialized_size(value: &impl Serialize, cap: usize) -> Option<usize> {
    struct BudgetWriter {
        used: usize,
        cap: usize,
    }
    impl Write for BudgetWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.used = self
                .used
                .checked_add(bytes.len())
                .filter(|used| *used <= self.cap)
                .ok_or_else(|| io::Error::other("Work metadata limit exceeded"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BudgetWriter { used: 0, cap };
    serde_json::to_writer(&mut writer, value).ok()?;
    Some(writer.used)
}
