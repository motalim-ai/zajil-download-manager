//! What every test of the `zajil` command shares.

use std::process::Command;

/// The `zajil` command, for tests. It always runs with
/// `ZAJIL_NO_POWER_ACTIONS` set, so no test can shut down or put to sleep
/// the machine it runs on, whatever it asks. `power_guard.rs` checks that
/// every test starts `zajil` through here.
pub fn zajil_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_zajil"));
    command.env(zajil_platform::power::NO_POWER_ACTIONS, "1");
    command
}

/// A `zajil` a test started in the background: killed and waited for when
/// the test ends, also when it fails before its own `kill` (#50: a process
/// left running can keep connecting to a port number that a later test's
/// server reuses). Use it as the `Child` it holds.
#[allow(dead_code)] // Not every test file starts `zajil` in the background.
pub struct Running(pub std::process::Child);

impl std::ops::Deref for Running {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Running {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Already ended: kill fails harmlessly, and wait returns at once.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
