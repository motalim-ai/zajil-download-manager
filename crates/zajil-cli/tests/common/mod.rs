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
