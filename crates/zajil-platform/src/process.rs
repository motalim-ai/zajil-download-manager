//! Running a video tool as a tree of processes that ends with it, or with
//! Zajil (ADR-0009, section 4).
//!
//! yt-dlp's standalone program is two processes (a loader and Python),
//! and it starts ffmpeg and a JavaScript runtime under them. Ending only
//! the process Zajil started would leave the rest running, still able to
//! write the cookie file it was given. So every tool runs in a tree of its
//! own:
//!
//! - **Windows:** a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
//!   The process is created suspended and joins the job before its first
//!   instruction, so nothing it starts is ever outside. When Zajil ends,
//!   however it ends, the system closes the job's handle and ends the
//!   whole tree.
//! - **macOS and Linux:** a process group of its own, with `umask 077`, so
//!   whatever the tree creates is the user's alone. On Linux the first
//!   process also gets `PR_SET_PDEATHSIG` (`SIGTERM`, which the loader
//!   passes on to Python). That signal follows the *thread* that started
//!   the process, so every tool is started from one thread that lives as
//!   long as Zajil. It reaches only that first process, and macOS has no
//!   such signal at all. So every tree also carries a random token in its
//!   environment (`ZAJIL_TOOL_RUN`, which yt-dlp passes on to what it
//!   starts), and each run writes it down as a [`Record`]: the next start
//!   of Zajil ends every process of the user that carries it
//!   ([`end_leftover`]), even after the first one is gone and its id
//!   reused.
//!
//! [`Tree::wait`] returns once the first process has ended *and* the rest
//! of the tree is gone (ended if needed), so the caller may then remove
//! the run's folder knowing nothing writes there any more. On macOS and
//! Linux "gone" means no process of this user carries the run's token any
//! more (one that left the process group, by `setsid` for instance, is
//! found and ended that way too).
//!
//! **Process ids are never used once free.** On macOS and Linux the first
//! process is reaped only after the rest of its group is ended: until then
//! its zombie keeps the group's id ours, so a signal to the group never
//! reaches another program's group that took the id. After the reap,
//! [`Killer::kill`] sends nothing to the group. So the first process is
//! not handed out: its pipes are ([`Tree::take_stdout`]…), and
//! [`Tree::try_wait`] and [`Tree::wait`] reap it in that order.
//!
//! Everything here blocks (waits, process scans): async code calls it from
//! a blocking task.

use std::io;
use std::process::{Child, Command, ExitStatus};

/// A tool's running process tree. Dropping it ends the whole tree.
pub struct Tree {
    child: Child,
    imp: std::sync::Arc<imp::Group>,
    record: Record,
    /// How the first process ended, once it is reaped: then nothing of the
    /// tree is left, and its id is no longer ours.
    status: Option<ExitStatus>,
}

/// Ends a [`Tree`] from another thread or task (a stopped download) while
/// its owner reads its output. Ending a tree that already ended is fine,
/// and reaches nothing else. Blocking.
#[derive(Clone)]
pub struct Killer(std::sync::Arc<imp::Group>);

impl Killer {
    pub fn kill(&self) -> io::Result<()> {
        self.0.kill()
    }
}

/// The environment variable that marks every process of a tree.
pub const TOKEN_VAR: &str = "ZAJIL_TOOL_RUN";

/// What identifies a run's process tree after a crash: the random token
/// every process of the tree carries in its environment. Process ids are
/// not used: the first process may be gone and its id given to another
/// program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    token: u64,
}

impl Record {
    /// One line of text, for a file in the run's folder.
    pub fn to_line(self) -> String {
        format!("{:016x}\n", self.token)
    }

    pub fn from_line(line: &str) -> Option<Record> {
        let hex = line.strip_suffix('\n').unwrap_or(line);
        if hex.len() != 16 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        let token = u64::from_str_radix(hex, 16).ok()?;
        (token != 0).then_some(Record { token })
    }

    /// `ZAJIL_TOOL_RUN=<token>`, as it appears in an environment.
    #[cfg_attr(windows, allow(dead_code))]
    fn entry(self) -> String {
        format!("{TOKEN_VAR}={:016x}", self.token)
    }
}

/// Starts `command` as a tree of its own (see the module). The command's
/// pipes and environment are the caller's; it must not be a shell.
pub fn spawn(mut command: Command) -> io::Result<Tree> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    let record = Record {
        token: u64::from_le_bytes(bytes).max(1),
    };
    command.env(TOKEN_VAR, format!("{:016x}", record.token));
    let (child, imp) = imp::spawn(command, record)?;
    Ok(Tree {
        child,
        imp: std::sync::Arc::new(imp),
        record,
        status: None,
    })
}

impl Tree {
    /// The first process's standard output, if it was piped (once).
    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }

    /// The first process's standard error, if it was piped (once).
    pub fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// The first process's standard input, if it was piped (once).
    pub fn take_stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.child.stdin.take()
    }

    /// Something that ends this tree from elsewhere.
    pub fn killer(&self) -> Killer {
        Killer(self.imp.clone())
    }

    /// What identifies this tree after a crash (see [`end_leftover`]).
    pub fn record(&self) -> Record {
        self.record
    }

    /// Ends every process of the tree now, and waits until they are gone.
    pub fn kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        self.imp.kill()?;
        self.wait().map(|_| ())
    }

    /// Waits for the first process to end, then for the rest of the tree,
    /// ending whatever it left behind. Blocking.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        // Ended, but not reaped: its id stays ours while the rest is ended.
        imp::wait_ended(&mut self.child)?;
        self.imp.finish()?;
        let status = self.child.wait()?;
        self.status = Some(status);
        Ok(status)
    }

    /// Whether the first process has ended: if so, the rest of the tree is
    /// ended as by [`Tree::wait`], and its status returned.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        if !imp::has_ended(&mut self.child)? {
            return Ok(None);
        }
        self.wait().map(Some)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

/// Ends every process of the current user that carries `record`'s token,
/// if any still runs: at Zajil's start, before the folders of earlier runs
/// are removed. Nothing to do on Windows, where the job ended the tree
/// with Zajil.
pub fn end_leftover(record: Record) -> io::Result<()> {
    imp::end_leftover(record)
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod imp {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    use super::Record;

    pub struct Group {
        pgid: libc::pid_t,
        record: Record,
        /// The first process was reaped: the group's id is no longer ours,
        /// so nothing is sent to it any more. Held while a signal is sent,
        /// and set before the reap.
        reaped: std::sync::Mutex<bool>,
    }

    impl Group {
        /// Ends the group, and every process that carries the run's token
        /// (one that left the group too), until none is left.
        pub fn kill(&self) -> io::Result<()> {
            {
                let reaped = self.reaped.lock().unwrap_or_else(|e| e.into_inner());
                // SAFETY: killpg sends a signal to the group we created,
                // whose leader is not reaped yet (`reaped` is held), so the
                // id is still that group's.
                if !*reaped && unsafe { libc::killpg(self.pgid, libc::SIGKILL) } != 0 {
                    let err = io::Error::last_os_error();
                    if !nothing_left_to_signal(&err) {
                        return Err(err);
                    }
                }
            }
            end_leftover(self.record)
        }

        /// The first process has ended (not reaped): ends the rest, then
        /// marks it reaped, so the caller may reap it.
        pub fn finish(&self) -> io::Result<()> {
            self.kill()?;
            *self.reaped.lock().unwrap_or_else(|e| e.into_inner()) = true;
            Ok(())
        }
    }

    /// Whether `killpg` failed only because no live process is left in the
    /// group: ESRCH; and on macOS also EPERM, which Darwin returns for a
    /// group whose only members are zombies, as our leader is until it is
    /// reaped (Linux returns ESRCH then).
    fn nothing_left_to_signal(err: &io::Error) -> bool {
        let code = err.raw_os_error();
        code == Some(libc::ESRCH) || (cfg!(target_os = "macos") && code == Some(libc::EPERM))
    }

    /// Waits until `child` has ended, without reaping it.
    pub fn wait_ended(child: &mut Child) -> io::Result<()> {
        loop {
            match waitid(child, 0) {
                Ok(_) => return Ok(()),
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }

    /// Whether `child` has ended, without reaping it.
    pub fn has_ended(child: &mut Child) -> io::Result<bool> {
        waitid(child, libc::WNOHANG)
    }

    /// `waitid` for `child`'s end, never reaping it (`WNOWAIT`). `true`
    /// when it has ended.
    fn waitid(child: &mut Child, flags: libc::c_int) -> io::Result<bool> {
        // SAFETY: a zeroed siginfo_t is a valid buffer for waitid.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waits for our own child (not reaped: the Tree reaps it
        // only after this), into `info`.
        let done = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | flags,
            )
        };
        if done != 0 {
            return Err(io::Error::last_os_error());
        }
        // With WNOHANG and nothing ended, the pid stays 0.
        Ok(pid_of(&info) != 0)
    }

    #[cfg(target_os = "linux")]
    fn pid_of(info: &libc::siginfo_t) -> libc::pid_t {
        // SAFETY: waitid filled the SIGCHLD fields, or left them zeroed.
        unsafe { info.si_pid() }
    }

    #[cfg(not(target_os = "linux"))]
    fn pid_of(info: &libc::siginfo_t) -> libc::pid_t {
        info.si_pid
    }

    pub fn spawn(mut command: Command, record: Record) -> io::Result<(Child, Group)> {
        command.process_group(0);
        #[cfg(target_os = "linux")]
        // SAFETY: the closure runs in the child between fork and exec and
        // calls only async-signal-safe functions (umask, prctl, getppid).
        unsafe {
            let parent = libc::getpid();
            command.pre_exec(move || {
                libc::umask(0o077);
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // Zajil may have died between fork and prctl: then nobody
                // will send the signal, so do not start at all.
                // (An error from a code: building a message would allocate,
                // which is not safe between fork and exec.)
                if libc::getppid() != parent {
                    return Err(io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
        #[cfg(not(target_os = "linux"))]
        // SAFETY: umask is async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                libc::umask(0o077);
                Ok(())
            });
        }
        let child = spawner::spawn(command)?;
        let pgid = child.id() as libc::pid_t;
        Ok((
            child,
            Group {
                pgid,
                record,
                reaped: std::sync::Mutex::new(false),
            },
        ))
    }

    /// The thread every tool starts from (see the module): on Linux the
    /// death signal follows it, so it must live as long as Zajil.
    mod spawner {
        use std::io;
        use std::process::{Child, Command};
        use std::sync::mpsc::{Sender, channel};
        use std::sync::{Mutex, OnceLock};

        type Request = (Command, Sender<io::Result<Child>>);

        fn requests() -> &'static Mutex<Sender<Request>> {
            static REQUESTS: OnceLock<Mutex<Sender<Request>>> = OnceLock::new();
            REQUESTS.get_or_init(|| {
                let (send, receive) = channel::<Request>();
                std::thread::Builder::new()
                    .name("zajil-tool-spawner".into())
                    .spawn(move || {
                        for (mut command, reply) in receive {
                            let _ = reply.send(command.spawn());
                        }
                    })
                    .expect("the tool spawner thread starts");
                Mutex::new(send)
            })
        }

        pub fn spawn(command: Command) -> io::Result<Child> {
            let (reply, answer) = channel();
            requests()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .send((command, reply))
                .map_err(|_| io::Error::other("the tool spawner thread ended"))?;
            answer
                .recv()
                .map_err(|_| io::Error::other("the tool spawner thread ended"))?
        }
    }

    /// The ids of the processes the current user may see.
    #[cfg(target_os = "linux")]
    fn pids() -> io::Result<Vec<libc::pid_t>> {
        Ok(std::fs::read_dir("/proc")?
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect())
    }

    /// The environment of `pid`, if this user may read it.
    #[cfg(target_os = "linux")]
    fn environment(pid: libc::pid_t) -> Option<Vec<u8>> {
        std::fs::read(format!("/proc/{pid}/environ")).ok()
    }

    #[cfg(target_os = "macos")]
    fn pids() -> io::Result<Vec<libc::pid_t>> {
        // SAFETY: with a null buffer, proc_listallpids returns the count.
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return Err(io::Error::last_os_error());
        }
        // Room for processes started meanwhile.
        let mut pids = vec![0 as libc::pid_t; count as usize + 64];
        let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        // SAFETY: the buffer holds `bytes` bytes.
        let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if got < 0 {
            return Err(io::Error::last_os_error());
        }
        pids.truncate(got as usize);
        Ok(pids)
    }

    /// The arguments and environment of `pid` (`KERN_PROCARGS2`), if this
    /// user may read them.
    #[cfg(target_os = "macos")]
    fn environment(pid: libc::pid_t) -> Option<Vec<u8>> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut size: libc::size_t = 0;
        // SAFETY: a size query (null buffer), then a read of `size` bytes.
        unsafe {
            if libc::sysctl(
                mib.as_mut_ptr(),
                3,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            ) != 0
            {
                return None;
            }
            let mut buf = vec![0u8; size];
            if libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            ) != 0
            {
                return None;
            }
            buf.truncate(size);
            Some(buf)
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn pids() -> io::Result<Vec<libc::pid_t>> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn environment(_pid: libc::pid_t) -> Option<Vec<u8>> {
        None
    }

    pub fn end_leftover(record: Record) -> io::Result<()> {
        // NUL-separated entries: match a whole entry, never part of one.
        let needle = format!("\0{}\0", record.entry()).into_bytes();
        // SAFETY: getpid has no preconditions.
        let me = unsafe { libc::getpid() };
        // A process may start another while we end it, and one that was
        // sent SIGKILL may take a moment to go: look again until a pass
        // finds none, for 5 seconds at most.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let mut found = false;
            for pid in pids()? {
                if pid == me || pid <= 1 {
                    continue;
                }
                let Some(mut env) = environment(pid) else {
                    continue;
                };
                env.insert(0, 0);
                env.push(0);
                if env.windows(needle.len()).any(|w| w == needle.as_slice()) {
                    found = true;
                    // SAFETY: sending SIGKILL to a process of this user that
                    // carries the run's token.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
            }
            if !found {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the tool's processes did not end within 5 seconds",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

#[cfg(windows)]
pub(crate) use imp::{Group, spawn_alone};

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
        TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    use super::Record;

    /// An owned handle, closed when dropped.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: we own the handle and close it once.
            unsafe { CloseHandle(self.0) };
        }
    }

    // SAFETY: a job handle is a kernel handle; the job functions may be
    // called on it from any thread, and it is closed once, on drop.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}

    pub struct Group {
        job: Handle,
    }

    impl Group {
        /// Ends every process of the job, and waits until they are gone
        /// (they end asynchronously).
        pub fn kill(&self) -> io::Result<()> {
            // SAFETY: the job handle is ours and open.
            if unsafe { TerminateJobObject(self.job.0, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            self.wait_empty()
        }

        /// The first process has ended: ends the rest of the job.
        pub fn finish(&self) -> io::Result<()> {
            self.kill()
        }

        /// Waits until no process is left in the job (they end
        /// asynchronously after TerminateJobObject).
        fn wait_empty(&self) -> io::Result<()> {
            for _ in 0..500 {
                // SAFETY: a plain C struct, valid when zeroed.
                let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION =
                    unsafe { std::mem::zeroed() };
                // SAFETY: the buffer is the size the call is told.
                let ok = unsafe {
                    QueryInformationJobObject(
                        self.job.0,
                        JobObjectBasicAccountingInformation,
                        (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                        std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                if info.ActiveProcesses == 0 {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the tool's processes did not end within 5 seconds",
            ))
        }
    }

    /// A new job whose processes end when its last handle closes; with
    /// `alone`, it may hold one process only (it cannot start another).
    fn new_job(alone: bool) -> io::Result<Handle> {
        // SAFETY: no attributes, no name: a new, unnamed job.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Handle(job);
        // SAFETY: a plain C struct, valid when zeroed.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if alone {
            limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = 1;
        }
        // SAFETY: the buffer is the size the call is told.
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Resumes every thread of the suspended process `pid` (it has one:
    /// the one created suspended).
    fn resume(pid: u32) -> io::Result<()> {
        // SAFETY: a snapshot of all threads; closed by Handle.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = Handle(snapshot);
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut resumed = 0;
        // SAFETY: `entry` is a THREADENTRY32 with its size set.
        let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: opening a thread of our own child to resume it.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = Handle(thread);
                // SAFETY: the handle has THREAD_SUSPEND_RESUME.
                if unsafe { ResumeThread(thread.0) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                resumed += 1;
            }
            // SAFETY: as above.
            more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
        }
        if resumed == 0 {
            return Err(io::Error::other("the tool's first thread was not found"));
        }
        Ok(())
    }

    /// Waits until `child` has ended. Its handle, held by `Child` until
    /// the tree is dropped, keeps its id ours: nothing to keep apart.
    pub fn wait_ended(child: &mut Child) -> io::Result<()> {
        child.wait().map(|_| ())
    }

    pub fn has_ended(child: &mut Child) -> io::Result<bool> {
        Ok(child.try_wait()?.is_some())
    }

    pub fn spawn(command: Command, _record: Record) -> io::Result<(Child, Group)> {
        spawn_in_job(command, false)
    }

    /// Starts `command` alone in a job of its own: it can start no other
    /// process (`zajil_platform::sandbox`).
    pub fn spawn_alone(command: Command) -> io::Result<(Child, Group)> {
        spawn_in_job(command, true)
    }

    fn spawn_in_job(mut command: Command, alone: bool) -> io::Result<(Child, Group)> {
        let job = new_job(alone)?;
        command.creation_flags(CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        let joined = (|| {
            // SAFETY: both handles are open; the child has not run yet.
            if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) } == 0 {
                return Err(io::Error::last_os_error());
            }
            resume(child.id())
        })();
        if let Err(err) = joined {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
        Ok((child, Group { job }))
    }

    pub fn end_leftover(_record: Record) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_reads_back_what_it_wrote_and_nothing_else() {
        let record = Record {
            token: 0x0123_4567_89ab_cdef,
        };
        assert_eq!(record.to_line(), "0123456789abcdef\n");
        assert_eq!(Record::from_line(&record.to_line()), Some(record));
        for bad in [
            "",
            "0123",
            "0123456789ABCDEF",
            "0000000000000000",
            "0123456789abcdefa",
            " 123456789abcdef",
            "0123456789abcdeg",
        ] {
            assert_eq!(Record::from_line(bad), None, "{bad:?}");
        }
    }
}
