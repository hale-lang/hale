//! A host's sudo, asked for its password where the person sees it.
//!
//! A host command that runs as root (`root = true`), and the packages a
//! host lacks, run under the person's sudo on the host. Over `ssh -tt` the
//! remote sudo's prompt comes back through the session's output, and a
//! terminal whose output is not on the screen (piped, captured, logged)
//! never shows it: the run sat silent, sudo waiting for a password no one
//! knew it asked for. So before a host's sudo is needed, it is asked over
//! the same login whether it needs a password (`sudo -n true`):
//!
//! - it needs none: the script runs as it always has;
//! - it needs one and this is a terminal: the password is asked for here,
//!   on the terminal itself (`/dev/tty`, not echoed), naming the login and
//!   the host, checked with a `sudo -k -S true` of its own (a wrong one
//!   asked again, three tries), and handed to the run's own `sudo -k -S`
//!   as the first line of its stdin: never in an argument, a variable, a
//!   file or a log, and the buffer zeroed when it is dropped;
//! - it needs one and there is no terminal: refused at once, naming the
//!   host, before anything runs there.

use std::io::{Read, Write};

/// The script that says what a login's sudo needs: `sudo free` (nothing,
/// a `NOPASSWD` rule or a cached ticket), `sudo password`, or `sudo none`
/// (no sudo there).
pub(super) const PROBE: &str = "command -v sudo >/dev/null 2>&1 || { echo 'sudo none'; exit 0; }; if sudo -n true >/dev/null 2>&1; then echo 'sudo free'; else echo 'sudo password'; fi";

/// The sudo a run uses when it hands the password over its stdin: the
/// cached ticket ignored (`-k`), so it reads the line it is handed whatever
/// an earlier sudo left behind, and no prompt of its own (`-p ''`).
pub(super) const FED: &str = "sudo -k -S -p ''";

/// The script that checks a password handed on its stdin, and leaves no
/// ticket behind. Its messages are sudo's in the C locale, so a wrong
/// password is told from any other refusal.
pub(super) const CHECK: &str = "LC_ALL=C sudo -k -S -p '' true";

/// How many times a wrong password is asked for again.
pub(super) const TRIES: usize = 3;

/// What a login's sudo needs, as `PROBE` answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Needs {
    Nothing,
    Password,
    NoSudo,
}

pub(super) fn needs(out: &str) -> Needs {
    match out.lines().rev().find_map(|l| l.trim().strip_prefix("sudo ")) {
        Some("free") => Needs::Nothing,
        Some("none") => Needs::NoSudo,
        _ => Needs::Password,
    }
}

/// What to do about a login's sudo before a script that needs it runs.
#[derive(Debug, PartialEq)]
pub(super) enum Decision {
    /// it needs no password: run as ever
    AsEver,
    /// ask the person for it here
    Ask,
    /// refused, before anything runs there
    Refuse(String),
}

/// What a login (`login`, `user@host`) whose sudo `needs` that, on `host`,
/// comes to, with or without a terminal here.
pub(super) fn decide(needs: Needs, tty: bool, login: &str, host: &str) -> Decision {
    match (needs, tty) {
        (Needs::Nothing, _) => Decision::AsEver,
        (Needs::NoSudo, _) => Decision::Refuse(format!("{login} has no sudo on {host}, and this runs as root through it: nothing was run there")),
        (Needs::Password, true) => Decision::Ask,
        (Needs::Password, false) => Decision::Refuse(format!(
            "sudo on {host} needs {login}'s password, and there is no terminal here to ask for it: nothing was run there (a NOPASSWD rule for the login in {host}'s sudoers, or running this at a terminal, lets it run)"
        )),
    }
}

/// What a failed `CHECK` said: `true` when the password was wrong (asked
/// again), `false` for any other refusal (the login is no sudoer, sudo
/// wants a terminal), which no other password mends.
pub(super) fn wrong_password(stderr: &str) -> bool {
    stderr.contains("incorrect password") || stderr.contains("Sorry, try again")
}

/// A password, held for one run and zeroed when it is dropped. Its buffer
/// is allocated whole up front, so nothing reallocates it and leaves a copy
/// behind.
pub(super) struct Password {
    buf: Vec<u8>,
    len: usize,
}

/// The longest password read; the rest of a longer line is dropped.
const LONGEST: usize = 1024;

impl Password {
    fn new() -> Password {
        Password { buf: vec![0u8; LONGEST], len: 0 }
    }

    #[cfg(test)]
    pub(super) fn of(s: &str) -> Password {
        let mut p = Password::new();
        for b in s.bytes() {
            p.push(b);
        }
        p
    }

    fn push(&mut self, b: u8) {
        if self.len < LONGEST {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    fn pop(&mut self) {
        if self.len > 0 {
            self.len -= 1;
            self.buf[self.len] = 0;
        }
    }

    fn clear(&mut self) {
        while self.len > 0 {
            self.pop();
        }
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Written to `w` as the first line of its stdin.
    pub(super) fn feed(&self, w: &mut impl Write) -> std::io::Result<()> {
        w.write_all(self.bytes())?;
        w.write_all(b"\n")?;
        w.flush()
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        for b in self.buf.iter_mut() {
            // SAFETY: a valid, aligned &mut u8; volatile so the zeroing of a
            // buffer about to be freed is not optimized away
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(..)")
    }
}

/// The terminal's settings, put back however the read ends.
struct Restore {
    fd: i32,
    was: libc::termios,
}

impl Drop for Restore {
    fn drop(&mut self) {
        // SAFETY: `fd` is the open terminal and `was` its settings as read
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.was) };
    }
}

/// A password asked for on this process's terminal (`/dev/tty`, whatever
/// its stdin and stdout are), after `prompt`, with nothing echoed: read a
/// key at a time, so erase and kill work and an interrupt (Ctrl-C) or an
/// end (Ctrl-D on nothing) gives up, the terminal put back either way.
pub(super) fn ask(prompt: &str) -> Result<Password, String> {
    use std::os::fd::AsRawFd;
    let mut tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").map_err(|e| format!("the terminal could not be opened to ask for the password: {e}"))?;
    let fd = tty.as_raw_fd();
    // SAFETY: a zeroed termios is a valid value for tcgetattr to fill
    let mut was: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is open, `was` a valid termios
    if unsafe { libc::tcgetattr(fd, &mut was) } != 0 {
        return Err("the terminal's settings could not be read to ask for the password".to_string());
    }
    let mut quiet = was;
    quiet.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG);
    quiet.c_cc[libc::VMIN] = 1;
    quiet.c_cc[libc::VTIME] = 0;
    tty.write_all(prompt.as_bytes()).and_then(|_| tty.flush()).map_err(|e| format!("the terminal: {e}"))?;
    // SAFETY: `fd` is open, `quiet` a valid termios
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) } != 0 {
        return Err("the terminal could not be set not to echo the password".to_string());
    }
    let restore = Restore { fd, was };
    let mut pw = Password::new();
    let mut key = [0u8; 1];
    let given = loop {
        match tty.read(&mut key) {
            Ok(0) => break pw.len > 0,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break false,
        }
        match key[0] {
            b'\n' | b'\r' => break true,
            // Ctrl-C
            0x03 => break false,
            // Ctrl-D: an end on nothing, else nothing
            0x04 if pw.len == 0 => break false,
            0x04 => {}
            // erase
            0x7f | 0x08 => pw.pop(),
            // kill: the line so far
            0x15 => pw.clear(),
            b => pw.push(b),
        }
    };
    // SAFETY: a valid &mut u8; the last key read, zeroed as the password is
    unsafe { std::ptr::write_volatile(&mut key[0], 0) };
    drop(restore);
    let _ = tty.write_all(b"\n");
    if !given {
        return Err("no password was given: nothing was run there".to_string());
    }
    Ok(pw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe's answer, as a login's shell prints it: its last word.
    #[test]
    fn the_probe_says_what_sudo_needs() {
        assert_eq!(needs("sudo free\n"), Needs::Nothing);
        assert_eq!(needs("motd noise\nsudo password\n"), Needs::Password);
        assert_eq!(needs("sudo none\n"), Needs::NoSudo);
        // an answer it cannot read is a password needed: asked, never assumed free
        assert_eq!(needs(""), Needs::Password);
        // the probe is a script `sh` reads
        assert!(std::process::Command::new("sh").args(["-n", "-c", PROBE]).status().unwrap().success());
        assert!(std::process::Command::new("sh").args(["-n", "-c", CHECK]).status().unwrap().success());
    }

    /// The probe itself, run by `sh` against a stand-in sudo: one that
    /// lets `-n` through is free, one that refuses it needs a password,
    /// and a login with no sudo on its PATH says so.
    #[test]
    fn the_probe_tells_the_three_logins_apart() {
        let run = |prelude: &str, path: Option<&str>| {
            // `/bin/sh` by its path: the child's PATH is the one looked in
            let mut c = std::process::Command::new("/bin/sh");
            c.args(["-c", &format!("{prelude}{PROBE}")]);
            if let Some(p) = path {
                c.env("PATH", p);
            }
            let out = c.output().unwrap();
            assert!(out.status.success());
            needs(&String::from_utf8_lossy(&out.stdout))
        };
        assert_eq!(run("sudo() { [ \"$1\" = -n ] || exit 2; return 0; }; ", None), Needs::Nothing);
        assert_eq!(run("sudo() { echo 'sudo: a password is required' >&2; return 1; }; ", None), Needs::Password);
        assert_eq!(run("", Some("/nonexistent-hale-probe-path")), Needs::NoSudo);
    }

    /// No password needed: as ever, terminal or not. A password needed:
    /// asked for at a terminal, refused off one at once, naming the host,
    /// what mends it, and that nothing ran.
    #[test]
    fn a_password_off_a_terminal_is_refused_naming_the_host() {
        for tty in [true, false] {
            assert_eq!(decide(Needs::Nothing, tty, "op@node-a", "node-a"), Decision::AsEver);
        }
        assert_eq!(decide(Needs::Password, true, "op@node-a", "node-a"), Decision::Ask);
        let Decision::Refuse(why) = decide(Needs::Password, false, "op@node-a", "node-a") else { panic!("refused off a terminal") };
        assert!(why.contains("sudo on node-a needs op@node-a's password") && why.contains("no terminal") && why.contains("NOPASSWD") && why.contains("at a terminal") && why.contains("nothing was run there"), "{why}");
        for tty in [true, false] {
            let Decision::Refuse(why) = decide(Needs::NoSudo, tty, "op@node-a", "node-a") else { panic!("no sudo is refused") };
            assert!(why.contains("no sudo on node-a") && why.contains("nothing was run there"), "{why}");
        }
    }

    /// A wrong password is asked again; any other refusal is not.
    #[test]
    fn a_wrong_password_is_told_from_another_refusal() {
        assert!(wrong_password("Sorry, try again.\nsudo: 1 incorrect password attempt\n"));
        assert!(!wrong_password("op is not in the sudoers file.  This incident will be reported.\n"));
        assert!(!wrong_password("sudo: sorry, you must have a tty to run sudo\n"));
    }

    /// The password is handed as one line, the buffer whole up front and
    /// never grown past it.
    #[test]
    fn the_password_is_one_line_of_its_own() {
        let p = Password::of("s3cr3t");
        let mut out = Vec::new();
        p.feed(&mut out).unwrap();
        assert_eq!(out, b"s3cr3t\n");
        assert_eq!(format!("{p:?}"), "Password(..)", "never printed");
        let long = Password::of(&"x".repeat(LONGEST + 10));
        assert_eq!(long.bytes().len(), LONGEST);
        let mut edited = Password::of("ab");
        edited.pop();
        edited.push(b'c');
        assert_eq!(edited.bytes(), b"ac");
        edited.clear();
        assert!(edited.bytes().is_empty());
    }

    /// `sudo -S` reads exactly the first line: what follows is the
    /// script's own stdin. A stand-in for sudo that reads its line as sudo
    /// does (a byte at a time) shows the script's stdin is untouched.
    #[test]
    fn the_script_keeps_the_rest_of_its_stdin() {
        let fake = r#"sudo() { IFS= read -r pw; [ "$pw" = s3cr3t ] || exit 1; shift 4; "$@"; }; sudo -k -S -p '' sh -c 'cat'"#;
        let mut c = std::process::Command::new("sh").args(["-c", fake]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        let mut w = c.stdin.take().unwrap();
        Password::of("s3cr3t").feed(&mut w).unwrap();
        w.write_all(b"the script's own\n").unwrap();
        drop(w);
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "the script's own\n");
        assert!(FED.starts_with("sudo -k -S -p ''"));
    }
}
