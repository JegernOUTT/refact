use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;

use super::{ObservationStatus, ObservedAccess};

const START_TIMEOUT: Duration = Duration::from_secs(2);
const SETUP_TRACEME_FAILED: u8 = 1;
const SETUP_FILTER_FAILED: u8 = 2;
const SECCOMP_DATA_NR_OFFSET: u32 = 0;
const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;
const BPF_LD_W_ABS: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
const BPF_JMP_JEQ_K: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
const BPF_RET_K: u16 = (libc::BPF_RET | libc::BPF_K) as u16;

#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xc000_003e;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xc000_00b7;
#[cfg(target_arch = "x86_64")]
const SYS_OPEN: u32 = libc::SYS_open as u32;
#[cfg(target_arch = "aarch64")]
const SYS_OPEN: u32 = u32::MAX;

#[derive(Debug)]
pub(crate) struct Setup {
    requested: bool,
    read_fd: Option<RawFd>,
    write_fd: Option<RawFd>,
    unavailable: Option<String>,
}

impl Setup {
    pub(crate) fn disabled() -> Self {
        Self {
            requested: false,
            read_fd: None,
            write_fd: None,
            unavailable: None,
        }
    }

    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            requested: true,
            read_fd: None,
            write_fd: None,
            unavailable: Some(reason.into()),
        }
    }

    pub(crate) fn prepare(command: &mut tokio::process::Command) -> Self {
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = command;
            return Self::unavailable("Linux observation is unsupported on this architecture");
        }

        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        {
            let mut fds = [-1; 2];
            if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
                return Self::unavailable(format!(
                    "failed to create observer setup pipe: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let read_fd = fds[0];
            let write_fd = fds[1];
            unsafe {
                command.as_std_mut().pre_exec(move || {
                    if libc::ptrace(
                        libc::PTRACE_TRACEME,
                        0,
                        std::ptr::null_mut::<libc::c_void>(),
                        std::ptr::null_mut::<libc::c_void>(),
                    ) == -1
                    {
                        write_setup_code(write_fd, SETUP_TRACEME_FAILED);
                        return Ok(());
                    }
                    if install_filter() != 0 {
                        write_setup_code(write_fd, SETUP_FILTER_FAILED);
                    }
                    Ok(())
                });
            }
            Self {
                requested: true,
                read_fd: Some(read_fd),
                write_fd: Some(write_fd),
                unavailable: None,
            }
        }
    }

    pub(crate) fn start(mut self, process_id: Option<u32>) -> Runtime {
        if !self.requested {
            return Runtime::unavailable("disabled");
        }
        if let Some(reason) = self.unavailable.take() {
            return Runtime::unavailable(reason);
        }
        close_fd(self.write_fd.take());
        let Some(process_id) = process_id else {
            close_fd(self.read_fd.take());
            return Runtime::unavailable("spawned process has no process id");
        };
        let setup_code = match read_setup_code(self.read_fd.take()) {
            Ok(code) => code,
            Err(reason) => return Runtime::unavailable(reason),
        };
        match setup_code {
            Some(SETUP_TRACEME_FAILED) => Runtime::unavailable("PTRACE_TRACEME is unavailable"),
            Some(SETUP_FILTER_FAILED) => {
                resume_unobserved(process_id as libc::pid_t);
                Runtime::unavailable("seccomp observation filter is unavailable")
            }
            Some(_) => {
                resume_unobserved(process_id as libc::pid_t);
                Runtime::unavailable("observer setup returned an unknown status")
            }
            None => {
                let process_id = process_id as libc::pid_t;
                if let Err(reason) = transfer_to_supervisor(process_id) {
                    fail_open_resume(process_id);
                    return Runtime::unavailable(reason);
                }
                start_supervisor(process_id)
            }
        }
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        close_fd(self.read_fd.take());
        close_fd(self.write_fd.take());
    }
}

#[derive(Clone)]
pub(crate) struct Handle {
    shared: Arc<Shared>,
}

impl Handle {
    pub(crate) async fn wait_exit(&self) -> Result<Option<i32>, String> {
        loop {
            let notified = self.shared.changed.notified();
            if let Some(result) = self.shared.state.lock().unwrap().exit.clone() {
                return result;
            }
            notified.await;
        }
    }

    pub(crate) fn try_exit(&self) -> Option<Result<Option<i32>, String>> {
        self.shared.state.lock().unwrap().exit.clone()
    }

    pub(crate) async fn wait_status(&self) -> ObservationStatus {
        loop {
            let notified = self.shared.changed.notified();
            if let Some(status) = self.shared.state.lock().unwrap().status.clone() {
                return status;
            }
            notified.await;
        }
    }
}

pub(crate) struct Runtime {
    handle: Option<Handle>,
    unavailable: Option<String>,
}

impl Runtime {
    fn active(handle: Handle) -> Self {
        Self {
            handle: Some(handle),
            unavailable: None,
        }
    }

    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            handle: None,
            unavailable: Some(reason.into()),
        }
    }

    pub(crate) fn handle(&self) -> Option<Handle> {
        self.handle.clone()
    }

    pub(crate) async fn finish(self, terminal: bool) -> ObservationStatus {
        match self.handle {
            Some(handle) if terminal => handle.wait_status().await,
            Some(_) => ObservationStatus::Unavailable(
                "observation is pending until the process exits".to_string(),
            ),
            None => ObservationStatus::Unavailable(
                self.unavailable
                    .unwrap_or_else(|| "backend unavailable".to_string()),
            ),
        }
    }
}

#[derive(Default)]
struct SharedState {
    exit: Option<Result<Option<i32>, String>>,
    status: Option<ObservationStatus>,
}

struct Shared {
    state: Mutex<SharedState>,
    changed: Notify,
}

impl Shared {
    fn new() -> Self {
        Self {
            state: Mutex::new(SharedState::default()),
            changed: Notify::new(),
        }
    }

    fn set_exit(&self, exit: Result<Option<i32>, String>) {
        let mut state = self.state.lock().unwrap();
        if state.exit.is_none() {
            state.exit = Some(exit);
            drop(state);
            self.changed.notify_waiters();
        }
    }

    fn set_status(&self, status: ObservationStatus) {
        let mut state = self.state.lock().unwrap();
        if state.status.is_none() {
            state.status = Some(status);
            drop(state);
            self.changed.notify_waiters();
        }
    }
}

#[derive(Clone, Copy)]
struct PendingOpen {
    reads: bool,
    writes: bool,
}

struct Supervisor {
    root: libc::pid_t,
    tracees: HashSet<libc::pid_t>,
    pending: HashMap<libc::pid_t, PendingOpen>,
    reads: HashSet<PathBuf>,
    writes: HashSet<PathBuf>,
    failure: Option<String>,
    shared: Arc<Shared>,
}

impl Supervisor {
    fn new(root: libc::pid_t, shared: Arc<Shared>) -> Self {
        Self {
            root,
            tracees: HashSet::from([root]),
            pending: HashMap::new(),
            reads: HashSet::new(),
            writes: HashSet::new(),
            failure: None,
            shared,
        }
    }

    fn run(mut self, ready: std::sync::mpsc::SyncSender<Result<(), String>>) {
        if let Err(reason) = seize_tracee(self.root)
            .and_then(|_| interrupt_tracee(self.root))
            .and_then(|_| wait_for_initial_stop(self.root))
            .and_then(|_| continue_tracee(self.root, 0))
        {
            fail_open_resume(self.root);
            let _ = ready.send(Err(reason.clone()));
            self.shared
                .set_status(ObservationStatus::Unavailable(reason));
            return;
        }
        unsafe {
            libc::kill(self.root, libc::SIGCONT);
        }
        if ready.send(Ok(())).is_err() {
            detach_stopped(self.root);
            return;
        }

        while !self.tracees.is_empty() {
            let mut status = 0;
            let process_id =
                unsafe { libc::waitpid(-1, &mut status, libc::__WALL | libc::__WNOTHREAD) };
            if process_id == -1 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                if error.raw_os_error() != Some(libc::ECHILD) {
                    self.fail(format!("observer waitpid failed: {error}"));
                }
                break;
            }
            self.handle_status(process_id, status);
        }

        if self.shared.state.lock().unwrap().exit.is_none() {
            let reason = self
                .failure
                .clone()
                .unwrap_or_else(|| "observer lost the root process".to_string());
            self.shared.set_exit(Err(reason));
        }
        let status = match self.failure {
            Some(reason) => ObservationStatus::Unavailable(reason),
            None => {
                let mut reads: Vec<_> = self.reads.into_iter().collect();
                let mut writes: Vec<_> = self.writes.into_iter().collect();
                reads.sort();
                writes.sort();
                ObservationStatus::Observed(ObservedAccess { reads, writes })
            }
        };
        self.shared.set_status(status);
    }

    fn handle_status(&mut self, process_id: libc::pid_t, status: libc::c_int) {
        if libc::WIFEXITED(status) {
            if process_id == self.root {
                self.shared.set_exit(Ok(Some(libc::WEXITSTATUS(status))));
            }
            self.tracees.remove(&process_id);
            self.pending.remove(&process_id);
            return;
        }
        if libc::WIFSIGNALED(status) {
            if process_id == self.root {
                self.shared.set_exit(Ok(None));
            }
            self.tracees.remove(&process_id);
            self.pending.remove(&process_id);
            return;
        }
        if !libc::WIFSTOPPED(status) {
            return;
        }

        let signal = libc::WSTOPSIG(status);
        let event = status >> 16;
        if event == libc::PTRACE_EVENT_SECCOMP {
            match read_open(process_id) {
                Ok(open) => {
                    self.pending.insert(process_id, open);
                    self.resume_syscall(process_id, 0);
                }
                Err(reason) => {
                    self.fail(reason);
                    self.resume_cont(process_id, 0);
                }
            }
            return;
        }
        if matches!(
            event,
            libc::PTRACE_EVENT_FORK | libc::PTRACE_EVENT_VFORK | libc::PTRACE_EVENT_CLONE
        ) {
            match event_process_id(process_id) {
                Ok(child) => {
                    self.tracees.insert(child);
                }
                Err(reason) => self.fail(reason),
            }
            self.resume_cont(process_id, 0);
            return;
        }
        if event == libc::PTRACE_EVENT_STOP {
            self.resume_cont(process_id, 0);
            return;
        }
        if signal == libc::SIGTRAP | 0x80 {
            if let Some(open) = self.pending.remove(&process_id) {
                match returned_fd(process_id) {
                    Ok(Some(fd)) => {
                        if let Some(path) = resolve_fd(process_id, fd) {
                            if open.reads {
                                self.reads.insert(path.clone());
                            }
                            if open.writes {
                                self.writes.insert(path);
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(reason) => self.fail(reason),
                }
            }
            self.resume_cont(process_id, 0);
            return;
        }

        let delivered_signal = match signal {
            libc::SIGSTOP | libc::SIGTRAP => 0,
            signal => signal,
        };
        if self.pending.contains_key(&process_id) {
            self.resume_syscall(process_id, delivered_signal);
        } else {
            self.resume_cont(process_id, delivered_signal);
        }
    }

    fn resume_cont(&mut self, process_id: libc::pid_t, signal: libc::c_int) {
        if let Err(reason) = continue_tracee(process_id, signal) {
            if !is_missing_process_error(&reason) {
                self.fail(reason);
            }
        }
    }

    fn resume_syscall(&mut self, process_id: libc::pid_t, signal: libc::c_int) {
        if let Err(reason) = syscall_tracee(process_id, signal) {
            if !is_missing_process_error(&reason) {
                self.fail(reason);
            }
        }
    }

    fn fail(&mut self, reason: String) {
        if self.failure.is_none() {
            self.failure = Some(reason);
            self.pending.clear();
            for process_id in self.tracees.iter().copied() {
                let _ = continue_tracee(process_id, 0);
            }
        }
    }
}

fn start_supervisor(root: libc::pid_t) -> Runtime {
    let shared = Arc::new(Shared::new());
    let handle = Handle {
        shared: shared.clone(),
    };
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let spawn = std::thread::Builder::new()
        .name(format!("refact-observe-{root}"))
        .spawn(move || Supervisor::new(root, shared).run(ready_tx));
    if let Err(error) = spawn {
        fail_open_resume(root);
        return Runtime::unavailable(format!("failed to start observer supervisor: {error}"));
    }
    match ready_rx.recv_timeout(START_TIMEOUT) {
        Ok(Ok(())) => Runtime::active(handle),
        Ok(Err(reason)) => Runtime::unavailable(reason),
        Err(error) => {
            fail_open_resume(root);
            Runtime::unavailable(format!("observer supervisor did not start: {error}"))
        }
    }
}

fn read_setup_code(read_fd: Option<RawFd>) -> Result<Option<u8>, String> {
    let Some(read_fd) = read_fd else {
        return Ok(None);
    };
    let mut reader = unsafe { std::fs::File::from_raw_fd(read_fd) };
    let mut status = [0_u8; 1];
    match reader.read(&mut status) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(status[0])),
        Err(error) => Err(format!("failed to read observer setup status: {error}")),
    }
}

fn close_fd(fd: Option<RawFd>) {
    if let Some(fd) = fd {
        unsafe {
            libc::close(fd);
        }
    }
}

unsafe fn write_setup_code(fd: RawFd, code: u8) {
    let _ = libc::write(fd, &code as *const u8 as *const libc::c_void, 1);
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn install_filter() -> libc::c_int {
    let mut filters = [
        statement(BPF_LD_W_ABS, SECCOMP_DATA_ARCH_OFFSET),
        jump(BPF_JMP_JEQ_K, AUDIT_ARCH, 1, 0),
        statement(BPF_RET_K, libc::SECCOMP_RET_ALLOW),
        statement(BPF_LD_W_ABS, SECCOMP_DATA_NR_OFFSET),
        jump(BPF_JMP_JEQ_K, SYS_OPEN, 3, 0),
        jump(BPF_JMP_JEQ_K, libc::SYS_openat as u32, 2, 0),
        jump(BPF_JMP_JEQ_K, libc::SYS_openat2 as u32, 1, 0),
        statement(BPF_RET_K, libc::SECCOMP_RET_ALLOW),
        statement(BPF_RET_K, libc::SECCOMP_RET_TRACE),
    ];
    let program = libc::sock_fprog {
        len: filters.len() as u16,
        filter: filters.as_mut_ptr(),
    };
    if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
        return -1;
    }
    libc::prctl(
        libc::PR_SET_SECCOMP,
        libc::SECCOMP_MODE_FILTER,
        &program as *const libc::sock_fprog,
    )
}

const fn statement(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

fn wait_for_initial_stop(process_id: libc::pid_t) -> Result<(), String> {
    let mut status = 0;
    loop {
        let waited =
            unsafe { libc::waitpid(process_id, &mut status, libc::__WALL | libc::__WNOTHREAD) };
        if waited == process_id {
            if libc::WIFSTOPPED(status) {
                return Ok(());
            }
            return Err("observed process exited before its initial ptrace stop".to_string());
        }
        if waited == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(format!("failed to wait for initial ptrace stop: {error}"));
        }
    }
}

fn ptrace_options() -> libc::c_int {
    libc::PTRACE_O_TRACESYSGOOD
        | libc::PTRACE_O_TRACESECCOMP
        | libc::PTRACE_O_TRACEFORK
        | libc::PTRACE_O_TRACEVFORK
        | libc::PTRACE_O_TRACECLONE
}

fn seize_tracee(process_id: libc::pid_t) -> Result<(), String> {
    ptrace_call(
        libc::PTRACE_SEIZE,
        process_id,
        std::ptr::null_mut(),
        ptrace_options() as usize as *mut libc::c_void,
        "seize tracee",
    )
}

fn interrupt_tracee(process_id: libc::pid_t) -> Result<(), String> {
    ptrace_call(
        libc::PTRACE_INTERRUPT,
        process_id,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        "interrupt tracee",
    )
}

fn transfer_to_supervisor(process_id: libc::pid_t) -> Result<(), String> {
    wait_for_initial_stop(process_id)?;
    ptrace_call(
        libc::PTRACE_DETACH,
        process_id,
        std::ptr::null_mut(),
        libc::SIGSTOP as usize as *mut libc::c_void,
        "stop tracee for supervisor transfer",
    )
}

fn continue_tracee(process_id: libc::pid_t, signal: libc::c_int) -> Result<(), String> {
    ptrace_call(
        libc::PTRACE_CONT,
        process_id,
        std::ptr::null_mut(),
        signal as usize as *mut libc::c_void,
        "continue tracee",
    )
}

fn syscall_tracee(process_id: libc::pid_t, signal: libc::c_int) -> Result<(), String> {
    ptrace_call(
        libc::PTRACE_SYSCALL,
        process_id,
        std::ptr::null_mut(),
        signal as usize as *mut libc::c_void,
        "continue tracee to syscall exit",
    )
}

fn ptrace_call(
    request: libc::c_uint,
    process_id: libc::pid_t,
    address: *mut libc::c_void,
    data: *mut libc::c_void,
    operation: &str,
) -> Result<(), String> {
    if unsafe { libc::ptrace(request, process_id, address, data) } != -1 {
        return Ok(());
    }
    Err(format!(
        "failed to {operation} for process {process_id}: {}",
        std::io::Error::last_os_error()
    ))
}

fn event_process_id(process_id: libc::pid_t) -> Result<libc::pid_t, String> {
    let mut child = 0_usize;
    ptrace_call(
        libc::PTRACE_GETEVENTMSG,
        process_id,
        std::ptr::null_mut(),
        &mut child as *mut usize as *mut libc::c_void,
        "read ptrace child event",
    )?;
    Ok(child as libc::pid_t)
}

#[cfg(target_arch = "x86_64")]
fn read_open(process_id: libc::pid_t) -> Result<PendingOpen, String> {
    let registers = registers(process_id)?;
    let syscall = registers.orig_rax as libc::c_long;
    let flags = if syscall == libc::SYS_open {
        registers.rsi
    } else if syscall == libc::SYS_openat {
        registers.rdx
    } else if syscall == libc::SYS_openat2 {
        peek_word(process_id, registers.rdx)?
    } else {
        return Err(format!("unexpected traced syscall {syscall}"));
    };
    Ok(PendingOpen {
        reads: flags_indicate_read(flags),
        writes: flags_indicate_write(flags),
    })
}

#[cfg(target_arch = "aarch64")]
fn read_open(process_id: libc::pid_t) -> Result<PendingOpen, String> {
    let registers = registers(process_id)?;
    let syscall = registers.regs[8] as libc::c_long;
    let flags = if syscall == libc::SYS_openat {
        registers.regs[2]
    } else if syscall == libc::SYS_openat2 {
        peek_word(process_id, registers.regs[2])?
    } else {
        return Err(format!("unexpected traced syscall {syscall}"));
    };
    Ok(PendingOpen {
        reads: flags_indicate_read(flags),
        writes: flags_indicate_write(flags),
    })
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn read_open(_process_id: libc::pid_t) -> Result<PendingOpen, String> {
    Err("Linux observation is unsupported on this architecture".to_string())
}

fn flags_indicate_read(flags: u64) -> bool {
    (flags as libc::c_int & libc::O_ACCMODE) != libc::O_WRONLY
}

fn flags_indicate_write(flags: u64) -> bool {
    let flags = flags as libc::c_int;
    (flags & libc::O_ACCMODE) != libc::O_RDONLY
        || (flags & (libc::O_CREAT | libc::O_TRUNC | libc::O_APPEND)) != 0
}

fn returned_fd(process_id: libc::pid_t) -> Result<Option<libc::c_int>, String> {
    let result = syscall_result(process_id)?;
    if result < 0 {
        Ok(None)
    } else {
        Ok(Some(result as libc::c_int))
    }
}

#[cfg(target_arch = "x86_64")]
fn syscall_result(process_id: libc::pid_t) -> Result<i64, String> {
    Ok(registers(process_id)?.rax as i64)
}

#[cfg(target_arch = "aarch64")]
fn syscall_result(process_id: libc::pid_t) -> Result<i64, String> {
    Ok(registers(process_id)?.regs[0] as i64)
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn syscall_result(_process_id: libc::pid_t) -> Result<i64, String> {
    Err("Linux observation is unsupported on this architecture".to_string())
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn registers(process_id: libc::pid_t) -> Result<libc::user_regs_struct, String> {
    let mut registers = unsafe { std::mem::zeroed::<libc::user_regs_struct>() };
    let mut registers_io = libc::iovec {
        iov_base: &mut registers as *mut libc::user_regs_struct as *mut libc::c_void,
        iov_len: std::mem::size_of::<libc::user_regs_struct>(),
    };
    ptrace_call(
        libc::PTRACE_GETREGSET,
        process_id,
        1_usize as *mut libc::c_void,
        &mut registers_io as *mut libc::iovec as *mut libc::c_void,
        "read tracee registers",
    )?;
    Ok(registers)
}

fn peek_word(process_id: libc::pid_t, address: u64) -> Result<u64, String> {
    unsafe {
        *libc::__errno_location() = 0;
        let value = libc::ptrace(
            libc::PTRACE_PEEKDATA,
            process_id,
            address as usize as *mut libc::c_void,
            std::ptr::null_mut::<libc::c_void>(),
        );
        if value == -1 && *libc::__errno_location() != 0 {
            return Err(format!(
                "failed to read tracee memory for process {process_id}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(value as u64)
    }
}

fn resolve_fd(process_id: libc::pid_t, fd: libc::c_int) -> Option<PathBuf> {
    let path = std::fs::read_link(format!("/proc/{process_id}/fd/{fd}")).ok()?;
    path.is_absolute().then_some(path)
}

fn resume_unobserved(process_id: libc::pid_t) {
    let mut status = 0;
    let waited =
        unsafe { libc::waitpid(process_id, &mut status, libc::__WALL | libc::__WNOTHREAD) };
    if waited == process_id && libc::WIFSTOPPED(status) {
        detach_stopped(process_id);
    }
}

fn fail_open_resume(process_id: libc::pid_t) {
    detach_stopped(process_id);
    unsafe {
        libc::kill(process_id, libc::SIGCONT);
    }
}

fn detach_stopped(process_id: libc::pid_t) {
    unsafe {
        libc::ptrace(
            libc::PTRACE_DETACH,
            process_id,
            std::ptr::null_mut::<libc::c_void>(),
            std::ptr::null_mut::<libc::c_void>(),
        );
    }
}

fn is_missing_process_error(reason: &str) -> bool {
    reason.contains("No such process")
}
