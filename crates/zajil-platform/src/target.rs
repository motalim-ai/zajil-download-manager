//! The system and processor this build of Zajil runs on, for choosing the
//! right file of a tool to download (ADR-0009).

/// An operating system Zajil runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Windows,
    Macos,
    Linux,
}

/// A processor architecture Zajil is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86_64,
    Aarch64,
}

/// A system and an architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Target {
    pub os: Os,
    pub arch: Arch,
}

impl Target {
    /// The suffix of a program file there: `.exe` on Windows, none elsewhere.
    pub fn exe_suffix(self) -> &'static str {
        match self.os {
            Os::Windows => ".exe",
            Os::Macos | Os::Linux => "",
        }
    }
}

/// This build's target, or `None` on a system or processor Zajil does not
/// ship for (it still builds there, for development).
pub fn current() -> Option<Target> {
    let os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Macos
    } else if cfg!(target_os = "linux") {
        Os::Linux
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        Arch::X86_64
    } else if cfg!(target_arch = "aarch64") {
        Arch::Aarch64
    } else {
        return None;
    };
    Some(Target { os, arch })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ci_runner_is_a_shipped_target() {
        // The CI runs on Windows, macOS and Linux, x86_64 or arm64: all
        // targets Zajil ships for, so none may come out as unknown there.
        let target = current().expect("an unknown target");
        assert_eq!(target.exe_suffix() == ".exe", cfg!(windows));
    }
}
