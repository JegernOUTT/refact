use std::io::{self, Write};
use serde::Serialize;

pub(crate) fn exact_serialized_size(value: &impl Serialize) -> Result<usize, serde_json::Error> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| io::Error::other("Work metadata size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}

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
    let mut writer = BudgetWriter {
        used: 0,
        cap,
    };
    serde_json::to_writer(&mut writer, value).ok()?;
    Some(writer.used)
}
