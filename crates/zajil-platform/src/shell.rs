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
    command_lines(program, &[Arg::Quoted(arg)])
}

/// One argument of a printed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg<'a> {
    /// Untrusted text (a path or URL): always quoted.
    Quoted(&'a str),
    /// Fixed text of our own, such as an option name: written as it is.
    Plain(&'a str),
}

/// `program` and its `args`, quoted for the shells people use on this
/// system. A shell that cannot quote one of the arguments safely gets
/// `None`.
pub fn command_lines(program: &str, args: &[Arg<'_>]) -> Vec<ShellCommand> {
    imp::shells()
        .into_iter()
        .map(|(shell, quote)| {
            let quoted: Option<Vec<String>> = args
                .iter()
                .map(|arg| match arg {
                    Arg::Quoted(text) => quote(text),
                    Arg::Plain(text) => Some((*text).to_owned()),
                })
                .collect();
            ShellCommand {
                shell,
                command: quoted.map(|args| format!("{program} {}", args.join(" "))),
            }
        })
        .collect()
}

type Quote = fn(&str) -> Option<String>;

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

    pub fn shells() -> Vec<(&'static str, Quote)> {
        vec![("your shell", |arg| Some(quote_posix(arg)))]
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub fn shells() -> Vec<(&'static str, Quote)> {
        vec![
            ("PowerShell", |arg| Some(quote_powershell(arg))),
            ("cmd", quote_cmd),
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
    fn command_lines_quote_every_untrusted_argument() {
        let lines = command_lines(
            "zajil get",
            &[
                Arg::Quoted("https://h/$(touch PWNED).zip"),
                Arg::Plain("--dir"),
                Arg::Quoted("/tmp/it's"),
            ],
        );
        assert!(!lines.is_empty());
        if cfg!(unix) {
            assert_eq!(
                lines[0].command.as_deref(),
                Some(r"zajil get 'https://h/$(touch PWNED).zip' --dir '/tmp/it'\''s'")
            );
        } else {
            assert_eq!(
                lines[0].command.as_deref(),
                Some("zajil get 'https://h/$(touch PWNED).zip' --dir '/tmp/it''s'")
            );
            // cmd cannot protect `%`, so it gets no command at all.
            let lines = command_lines("zajil get", &[Arg::Quoted("%PATH%"), Arg::Quoted("x")]);
            assert_eq!(lines[1].command, None);
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
