//! A light sandbox for the JavaScript runtime that YouTube's pages need
//! (ADR-0009, owner's conditions for QuickJS-NG). One program runs in it,
//! named when the sandbox is entered; where the system allows it:
//!
//! - it may start no other program (review: a program it started would
//!   not be bound by the rest);
//! - it may write only in its own empty folder;
//! - it may not reach the network.
//!
//! It may still read what the user can read.
//!
//! - **Linux:** Landlock, on the calling thread and every program it starts
//!   (so [`Sandbox::enter`] refuses to run once other threads exist).
//!   Running anything but the program, and writing outside the folder, are
//!   refused from Linux 5.13 (truncating from 6.2); TCP from 6.7; abstract
//!   UNIX sockets and signals out of the sandbox from 6.12. Landlock does
//!   not cover UDP or connecting to a UNIX socket file: the program itself
//!   (QuickJS-NG) has no way to open a socket, and can start nothing that
//!   has.
//! - **macOS:** `sandbox-exec`, around the program only: no other program,
//!   no writing outside the folder, no network.
//! - **Windows:** the calling process is lowered to low integrity, and with
//!   it the program: it can no longer write where the user normally writes
//!   (but in the folder, labelled low). The program runs alone in a job
//!   that allows one process. The network is not refused.
//!
//! What a system does not allow is not an error: the program runs anyway,
//! and [`Confinement`] says what holds, for the log.
//!
//! [`Sandbox::enter`] changes the calling process for good on Linux and
//! Windows: it is for a process made for this (`zajil-jsbox`), never for
//! Zajil itself.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// What the sandbox holds on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confinement {
    /// Starting any program but the sandboxed one is refused.
    pub programs: bool,
    /// Writing outside the folder is refused.
    pub files: bool,
    /// Reaching the network is refused (on Linux: TCP; see the module).
    pub network: bool,
}

impl Confinement {
    #[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
    const NONE: Confinement = Confinement {
        programs: false,
        files: false,
        network: false,
    };
}

/// A sandbox around one folder and one program.
#[derive(Debug)]
pub struct Sandbox {
    work: PathBuf,
    program: PathBuf,
    confinement: Confinement,
}

/// The sandboxed program, running.
pub struct Confined {
    child: Child,
    /// Windows: the job that holds it, alone; closing it ends it.
    #[cfg(windows)]
    _job: crate::process::Group,
}

impl Confined {
    /// The program, for its pipes and its end.
    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Sandbox {
    /// Confines what this process starts from now on: only `program` may
    /// run, writing only in `work` (an existing folder of its own), and no
    /// network, where the system allows it. On Linux and Windows the
    /// calling process is confined too, and that cannot be undone; on
    /// Linux it must have no other thread yet.
    pub fn enter(work: &Path, program: &Path) -> io::Result<Sandbox> {
        let work = work.canonicalize()?;
        let program = program.canonicalize()?;
        let confinement = imp::enter(&work, &program)?;
        Ok(Sandbox {
            work,
            program,
            confinement,
        })
    }

    /// What holds on this system.
    pub fn confinement(&self) -> Confinement {
        self.confinement
    }

    /// The command that runs the program in the sandbox, in its folder.
    /// Start it with [`Sandbox::spawn`].
    pub fn command(&self) -> Command {
        let mut command = imp::command(&self.program, &self.work);
        command.current_dir(&self.work);
        command
    }

    /// Starts `command` (from [`Sandbox::command`]): on Windows, alone in a
    /// job that allows no other process; elsewhere as a plain child, in
    /// this process's group.
    pub fn spawn(&self, command: Command) -> io::Result<Confined> {
        #[cfg(windows)]
        {
            let (child, job) = crate::process::spawn_alone(command)?;
            Ok(Confined { child, _job: job })
        }
        #[cfg(not(windows))]
        {
            let mut command = command;
            Ok(Confined {
                child: command.spawn()?,
            })
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::path::Path;
    use std::process::Command;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, LandlockStatus, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus, Scope, path_beneath_rules,
    };

    use super::Confinement;

    /// Landlock confines the calling thread only (before ABI 8): other
    /// threads, and what they start, would stay free.
    fn single_threaded() -> io::Result<()> {
        let threads = std::fs::read_dir("/proc/self/task")?.count();
        if threads != 1 {
            return Err(io::Error::other(format!(
                "the sandbox must be entered before any other thread starts ({threads} threads)"
            )));
        }
        Ok(())
    }

    pub fn enter(work: &Path, program: &Path) -> io::Result<Confinement> {
        single_threaded()?;
        // The rights of the newest ABI this crate knows; an older kernel
        // enforces what it has (best effort).
        let abi = ABI::V6;
        let status = Ruleset::default()
            .handle_access(AccessFs::from_all(abi))
            .and_then(|r| r.handle_access(AccessNet::from_all(abi)))
            .and_then(|r| r.scope(Scope::AbstractUnixSocket | Scope::Signal))
            .and_then(|r| r.create())
            // Reading anything the user can, running nothing...
            .and_then(|r| {
                r.add_rules(path_beneath_rules(
                    ["/"],
                    AccessFs::ReadFile | AccessFs::ReadDir,
                ))
            })
            // ...but the program itself (QuickJS-NG's is static: no loader).
            .and_then(|r| {
                r.add_rules(path_beneath_rules(
                    [program],
                    AccessFs::ReadFile | AccessFs::Execute,
                ))
            })
            // Writing only in the folder (where running is refused too).
            .and_then(|r| {
                r.add_rules(path_beneath_rules(
                    [work],
                    AccessFs::from_all(abi) & !AccessFs::Execute,
                ))
            })
            // Where nothing goes: `Stdio::null()` opens it for writing.
            .and_then(|r| r.add_rules(path_beneath_rules(["/dev/null"], AccessFs::WriteFile)))
            // No TCP port at all: no rule.
            .and_then(|r| r.restrict_self())
            .map_err(io::Error::other)?;
        if status.ruleset == RulesetStatus::NotEnforced {
            return Ok(Confinement::NONE);
        }
        let network = matches!(
            status.landlock,
            LandlockStatus::Available { effective_abi, .. } if effective_abi >= ABI::V4
        );
        Ok(Confinement {
            programs: true,
            files: true,
            network,
        })
    }

    pub fn command(program: &Path, _work: &Path) -> Command {
        Command::new(program)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::io;
    use std::path::Path;
    use std::process::Command;

    use super::Confinement;

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

    pub fn enter(_work: &Path, _program: &Path) -> io::Result<Confinement> {
        // The program is wrapped by `command`; this process stays as it is.
        let there = Path::new(SANDBOX_EXEC).is_file();
        Ok(Confinement {
            programs: there,
            files: there,
            network: there,
        })
    }

    /// A string literal of the sandbox profile language.
    fn literal(path: &Path) -> String {
        let text = path.to_string_lossy();
        let mut out = String::with_capacity(text.len() + 2);
        out.push('"');
        for c in text.chars() {
            if c == '"' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    }

    /// The profile: everything as usual, but no other program (not even
    /// through `open`, which needs to run one), no network, no writing
    /// outside the folder. The last matching rule wins.
    pub fn profile(work: &Path, program: &Path) -> String {
        format!(
            "(version 1)\n(allow default)\n(deny network*)\n\
             (deny process-fork)\n(deny process-exec)\n\
             (allow process-exec (literal {}))\n\
             (deny file-write* (require-not (subpath {})))\n",
            literal(program),
            literal(work)
        )
    }

    pub fn command(program: &Path, work: &Path) -> Command {
        if !Path::new(SANDBOX_EXEC).is_file() {
            return Command::new(program);
        }
        let mut command = Command::new(SANDBOX_EXEC);
        command.arg("-p").arg(profile(work, program)).arg(program);
        command
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::process::Command;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
        SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        GetLengthSid, GetSecurityDescriptorSacl, LABEL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    };
    use windows_sys::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::Confinement;

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    /// Labels `work` low integrity (and what is created in it), so a low
    /// integrity process may write there.
    fn label_low(work: &Path) -> io::Result<()> {
        // A SACL with one mandatory label: low, inherited by files and
        // folders, no write up.
        let sddl = wide(std::ffi::OsStr::new("S:(ML;OICI;NW;;;LW)"));
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a valid SDDL string; the descriptor is freed below.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let (mut present, mut defaulted) = (0, 0);
            let mut sacl = std::ptr::null_mut();
            // SAFETY: the descriptor was made above.
            if unsafe {
                GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted)
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let path = wide(work.as_os_str());
            // SAFETY: a valid path and the SACL of the descriptor above.
            let status = unsafe {
                SetNamedSecurityInfoW(
                    path.as_ptr(),
                    SE_FILE_OBJECT,
                    LABEL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    sacl,
                )
            };
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            Ok(())
        })();
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(descriptor) };
        result
    }

    /// Lowers this process's token to low integrity: what it starts from
    /// now on is low integrity too. A process may lower itself, never raise.
    fn lower_self() -> io::Result<()> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: our own process's token, closed below.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_DEFAULT | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let low = wide(std::ffi::OsStr::new("S-1-16-4096"));
            let mut sid = std::ptr::null_mut();
            // SAFETY: a valid SID string; the SID is freed below.
            if unsafe { ConvertStringSidToSidW(low.as_ptr(), &mut sid) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let label = TOKEN_MANDATORY_LABEL {
                Label: SID_AND_ATTRIBUTES {
                    Sid: sid,
                    Attributes: SE_GROUP_INTEGRITY as u32,
                },
            };
            // SAFETY: the label points at a valid SID; the size counts the
            // SID too, as Microsoft's own example does.
            let ok = unsafe {
                SetTokenInformation(
                    token,
                    TokenIntegrityLevel,
                    (&label as *const TOKEN_MANDATORY_LABEL).cast(),
                    std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid),
                )
            };
            // SAFETY: allocated by ConvertStringSidToSidW.
            unsafe { LocalFree(sid) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        // SAFETY: opened above.
        unsafe { CloseHandle(token) };
        result
    }

    pub fn enter(work: &Path, _program: &Path) -> io::Result<Confinement> {
        label_low(work)?;
        lower_self()?;
        Ok(Confinement {
            programs: true,
            files: true,
            network: false,
        })
    }

    pub fn command(program: &Path, _work: &Path) -> Command {
        Command::new(program)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    use std::io;
    use std::path::Path;
    use std::process::Command;

    use super::Confinement;

    pub fn enter(_work: &Path, _program: &Path) -> io::Result<Confinement> {
        Ok(Confinement::NONE)
    }

    pub fn command(program: &Path, _work: &Path) -> Command {
        Command::new(program)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn the_profile_quotes_the_folder_and_the_program() {
        let profile = super::imp::profile(
            std::path::Path::new("/tmp/a\"b\\c"),
            std::path::Path::new("/opt/q\"js"),
        );
        assert!(profile.contains(r#"(subpath "/tmp/a\"b\\c")"#), "{profile}");
        assert!(
            profile.contains(r#"(allow process-exec (literal "/opt/q\"js"))"#),
            "{profile}"
        );
    }
}
