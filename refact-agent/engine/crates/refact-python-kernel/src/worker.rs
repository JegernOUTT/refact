use crate::{kernel::*, protocol::*};
use std::io::{self, Read, Write};
pub const INTERPRETER_STACK_SIZE: usize = 64 * 1024 * 1024;
struct Framed<R, W> {
    reader: R,
    writer: W,
}
impl<R: Read, W: Write> KernelIo for Framed<R, W> {
    fn emit(&mut self, e: KernelEvent) -> io::Result<()> {
        match write_frame(&mut self.writer, &e) {
            Err(err) if err.kind() == io::ErrorKind::InvalidData => write_frame(
                &mut self.writer,
                &KernelEvent::ProtocolError {
                    message: err.to_string(),
                },
            ),
            result => result,
        }
    }
    fn recv(&mut self) -> io::Result<KernelCommand> {
        loop {
            match read_frame(&mut self.reader) {
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                    self.emit(KernelEvent::ProtocolError {
                        message: e.to_string(),
                    })?
                }
                result => return result,
            }
        }
    }
}
pub fn run_worker(reader: impl Read, writer: impl Write, os: Box<dyn OsBackend>) -> io::Result<()> {
    let mut io = Framed {
        reader,
        writer,
    };
    io.emit(KernelEvent::Ready {
        monty_version: "1.6.0".into(),
    })?;
    let mut kernel = Kernel::new(os);
    loop {
        let cmd = match io.recv() {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        if !kernel.handle(cmd, &mut io)? {
            return Ok(());
        }
    }
}
pub fn run_worker_stdio() -> io::Result<()> {
    std::thread::Builder::new()
        .name("python-worker".into())
        .stack_size(INTERPRETER_STACK_SIZE)
        .spawn(|| {
            run_worker(
                io::stdin().lock(),
                io::stdout().lock(),
                Box::new(crate::hostfs::HostFs::default()),
            )
        })?
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("python worker thread panicked")))
}
