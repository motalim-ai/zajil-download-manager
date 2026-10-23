//! Printing a command that a user can copy and run safely.
//!
//! File names come from servers, so a printed path may contain `$(...)`,
//! backticks, `%VAR%` or quotes. Each shell needs its own quoting; inside
//! the quotes produced here, nothing is expanded or executed.

/// A ready-to-paste command line for one shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    /// The shell it is for, as shown to the user.
    pub shell: &'static str,
    /// The command, or `None` if this shell cannot quote the argument
    /// safely (the user should use another shell from the list).
    pub command: Option<String>,
}

/// `program arg`, quoted for the shells people use on this system: POSIX
/// shells (bash, zsh, sh) on macOS and Linux; PowerShell and cmd on Windows.
pub fn commands(program: &str, arg: &str) -> Vec<ShellCommand> {
    imp::commands(program, arg)
}

/// Commands that delete exactly the files at `paths`, quoted for the shells
/// people use on this system: `rm --` in POSIX shells, `Remove-Item
/// -LiteralPath` in PowerShell, `del` in cmd (`None` where cmd cannot quote a
/// path safely). Paths come from servers: printed bare, a name with a space
/// or `$(…)` would delete another file or run code when pasted.
pub fn delete_commands(paths: &[&str]) -> Vec<ShellCommand> {
    imp::delete_commands(paths)
}

/// bash, zsh and other POSIX shells: single quotes, where nothing is
/// special, with each `'` written as `'\''`.
pub fn quote_posix(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// PowerShell: single quotes, where nothing is expanded. PowerShell also
/// treats the typographic quotes ‘ ’ ‚ ‛ as single quotes, so every one of
/// them is doubled like `'`.
pub fn quote_powershell(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for c in arg.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// cmd: double quotes keep `& | < > ^ ( )` literal, but cmd still expands
/// `%VAR%` (and `!VAR!` with delayed expansion) inside them and has no
/// reliable escape there. So an argument with `%`, `!`, `"` or a control
/// character gets `None`: cmd cannot be given it safely.
pub fn quote_cmd(arg: &str) -> Option<String> {
    let unsafe_char = |c: char| matches!(c, '%' | '!' | '"') || c.is_control();
    (!arg.chars().any(unsafe_char)).then(|| format!("\"{arg}\""))
}

#[cfg(unix)]
mod imp {
    use super::*;

    pub fn commands(program: &str, arg: &str) -> Vec<ShellCommand> {
        vec![ShellCommand {
            shell: "your shell",
            command: Some(format!("{program} {}", quote_posix(arg))),
        }]
    }

    pub fn delete_commands(paths: &[&str]) -> Vec<ShellCommand> {
        let quoted: Vec<String> = paths.iter().map(|p| quote_posix(p)).collect();
        vec![ShellCommand {
            shell: "your shell",
            command: Some(format!("rm -- {}", quoted.join(" "))),
        }]
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub fn commands(program: &str, arg: &str) -> Vec<ShellCommand> {
        vec![
            ShellCommand {
                shell: "PowerShell",
                command: Some(format!("{program} {}", quote_powershell(arg))),
            },
            ShellCommand {
                shell: "cmd",
                command: quote_cmd(arg).map(|quoted| format!("{program} {quoted}")),
            },
        ]
    }

    pub fn delete_commands(paths: &[&str]) -> Vec<ShellCommand> {
        let powershell: Vec<String> = paths.iter().map(|p| quote_powershell(p)).collect();
        let cmd: Option<Vec<String>> = paths.iter().map(|p| quote_cmd(p)).collect();
        vec![
            ShellCommand {
                shell: "PowerShell",
                command: Some(format!("Remove-Item -LiteralPath {}", powershell.join(","))),
            },
            ShellCommand {
                shell: "cmd",
                command: cmd.map(|quoted| format!("del {}", quoted.join(" "))),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NASTY: &str = "/tmp/$(touch PWNED)`id`.zip.zajil-state";

    #[test]
    fn posix_quoting_is_literal() {
        assert_eq!(quote_posix(NASTY), format!("'{NASTY}'"));
        assert_eq!(quote_posix("it's"), r"'it'\''s'");
        assert_eq!(quote_posix(""), "''");
    }

    #[test]
    fn powershell_quoting_doubles_every_single_quote() {
        assert_eq!(quote_powershell("a$(b)"), "'a$(b)'");
        assert_eq!(quote_powershell("it's"), "'it''s'");
        assert_eq!(quote_powershell("x\u{2019}y"), "'x\u{2019}\u{2019}y'");
    }

    #[test]
    fn cmd_quoting_refuses_what_it_cannot_protect() {
        assert_eq!(
            quote_cmd(r"C:\d\$(touch PWNED)`id` & x.zip").as_deref(),
            Some(r#""C:\d\$(touch PWNED)`id` & x.zip""#)
        );
        for bad in ["%PATH%.zip", "a!b!.zip", "a\"b", "a\nb"] {
            assert_eq!(quote_cmd(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn delete_commands_quote_every_path() {
        let commands = delete_commands(&[NASTY, "/tmp/My Report.pdf"]);
        assert!(!commands.is_empty());
        let first = commands[0].command.as_deref().unwrap();
        if cfg!(unix) {
            assert_eq!(first, format!("rm -- '{NASTY}' '/tmp/My Report.pdf'"));
        } else {
            assert_eq!(
                first,
                format!("Remove-Item -LiteralPath '{NASTY}','/tmp/My Report.pdf'")
            );
            assert_eq!(delete_commands(&["a%PATH%"])[1].command, None);
        }
    }

    #[test]
    fn commands_cover_this_system() {
        let commands = commands("zajil resume", NASTY);
        assert!(!commands.is_empty());
        assert!(
            commands[0]
                .command
                .as_deref()
                .unwrap()
                .starts_with("zajil resume '")
        );
    }
}
