//! Shutting the computer down or putting it to sleep (`PowerControl`,
//! ADR-0006), after a queue finishes (M3).
//!
//! | | Shut down | Sleep |
//! |---|---|---|
//! | Windows | `InitiateShutdownW` with `SHUTDOWN_POWEROFF`, after enabling `SeShutdownPrivilege`. Programs with unsaved work may still stop it | `SetSuspendState` |
//! | macOS | the "really shut down" Apple event to `loginwindow` (Apple's documented way, Technical Q&A QA1134), sent with `osascript` | `pmset sleepnow` |
//! | Linux | `systemctl poweroff`, through systemd-logind and its polkit rules | `systemctl suspend` |
//!
//! **Safety.** Zajil never calls this unless the user asked for it for one
//! queue, and only after a countdown the user can cancel. Tests never call
//! [`SystemPower`]: they use a mock, and a guard test checks it. As a last
//! fence, [`SystemPower`] refuses to act while the environment variable
//! [`NO_POWER_ACTIONS`] is set (CI sets it), so a mistake in a test cannot
//! turn a machine off. It can only stop an action, never cause one.
//!
//! None of this was run for real by an automated test: the manual steps
//! are in `docs/MANUAL-TESTS.md`.

use std::io;

/// While set (to anything), [`SystemPower`] refuses every action.
pub const NO_POWER_ACTIONS: &str = "ZAJIL_NO_POWER_ACTIONS";

/// What to do with the computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Shutdown,
    Sleep,
}

impl std::fmt::Display for PowerAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PowerAction::Shutdown => "shut down",
            PowerAction::Sleep => "sleep",
        })
    }
}

/// Shuts the computer down or puts it to sleep. One implementation per
/// system ([`SystemPower`]); tests use their own.
pub trait PowerControl: Send + Sync {
    /// Asks the system for `action`. Returns once it is asked: the
    /// computer goes down (or to sleep) a moment later, or refuses.
    fn perform(&self, action: PowerAction) -> io::Result<()>;
}

/// The real thing, for this system.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPower;

impl PowerControl for SystemPower {
    fn perform(&self, action: PowerAction) -> io::Result<()> {
        if refused_by_environment() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("not done: {NO_POWER_ACTIONS} is set"),
            ));
        }
        imp::perform(action)
    }
}

/// Whether [`NO_POWER_ACTIONS`] forbids every action now.
pub fn refused_by_environment() -> bool {
    std::env::var_os(NO_POWER_ACTIONS).is_some()
}

/// Runs a system command and fails clearly if it does not succeed.
#[cfg(unix)]
fn run(program: &str, args: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|err| io::Error::new(err.kind(), format!("could not run {program}: {err}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{program} {} failed: {status}",
            args.join(" ")
        )))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::io;

    use super::{PowerAction, run};

    pub fn perform(action: PowerAction) -> io::Result<()> {
        match action {
            // kAEShutDown ('rsdn') to loginwindow: shut down without its own
            // confirmation (the countdown was ours).
            PowerAction::Shutdown => run(
                "/usr/bin/osascript",
                &["-e", "tell application \"loginwindow\" to «event aevtrsdn»"],
            ),
            PowerAction::Sleep => run("/usr/bin/pmset", &["sleepnow"]),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::io;

    use super::{PowerAction, run};

    pub fn perform(action: PowerAction) -> io::Result<()> {
        match action {
            PowerAction::Shutdown => run("systemctl", &["poweroff"]),
            PowerAction::Sleep => run("systemctl", &["suspend"]),
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Power::SetSuspendState;
    use windows_sys::Win32::System::Shutdown::{
        InitiateShutdownW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER,
        SHTDN_REASON_MINOR_OTHER, SHUTDOWN_POWEROFF,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::PowerAction;

    pub fn perform(action: PowerAction) -> io::Result<()> {
        enable_shutdown_privilege()?;
        match action {
            PowerAction::Shutdown => {
                // SAFETY: null machine name (this computer) and message are
                // allowed; the other arguments are plain values.
                let error = unsafe {
                    InitiateShutdownW(
                        std::ptr::null(),
                        std::ptr::null(),
                        0,
                        SHUTDOWN_POWEROFF,
                        SHTDN_REASON_FLAG_PLANNED
                            | SHTDN_REASON_MAJOR_OTHER
                            | SHTDN_REASON_MINOR_OTHER,
                    )
                };
                if error != 0 {
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
                Ok(())
            }
            PowerAction::Sleep => {
                // SAFETY: plain values; sleep, not hibernate, not forced,
                // wake events allowed.
                if unsafe { SetSuspendState(false, false, false) } {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        }
    }

    /// Both calls need `SeShutdownPrivilege`, which a user has but which
    /// is off until a process turns it on.
    fn enable_shutdown_privilege() -> io::Result<()> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo-handle of this process needs no closing;
        // `token` is a writable handle.
        let ok = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let mut luid = LUID::default();
            // SAFETY: a null system name means this computer; the name is
            // a NUL-terminated constant; `luid` is writable.
            if unsafe { LookupPrivilegeValueW(std::ptr::null(), SE_SHUTDOWN_NAME, &mut luid) } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            // SAFETY: `token` is open with TOKEN_ADJUST_PRIVILEGES and
            // `privileges` is a valid structure; the previous state is not
            // asked for.
            let ok = unsafe {
                AdjustTokenPrivileges(
                    token,
                    0,
                    &privileges,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            // It may succeed without granting the privilege, which the
            // last error then says.
            let error = io::Error::last_os_error();
            if ok == 0 || error.raw_os_error().is_some_and(|code| code != 0) {
                return Err(error);
            }
            Ok(())
        })();
        // SAFETY: the token was opened above and is closed once.
        unsafe { CloseHandle(token) };
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_have_names() {
        assert_eq!(PowerAction::Shutdown.to_string(), "shut down");
        assert_eq!(PowerAction::Sleep.to_string(), "sleep");
    }

    /// The fence for tests: CI sets the variable for every test run
    /// (`.github/workflows/ci.yml`). This only reads the switch; it never
    /// calls [`SystemPower`].
    #[test]
    fn ci_runs_tests_with_power_actions_forbidden() {
        if std::env::var_os("CI").is_some() {
            assert!(
                refused_by_environment(),
                "{NO_POWER_ACTIONS} must be set on CI"
            );
        }
    }
}
