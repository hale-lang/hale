use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// Bind a child's life to ours (GH #905).
///
/// `hale run` is a foreground wrapper: the program it compiled, the
/// iris session `--observe` puts beside it, the `hale build` that
/// materializes the observer, fuse-hl under `hale iris`. None of them
/// has a reason to outlive the `hale` that asked for it, and when one
/// does it is not merely a stray — it inherited our descriptors, so a
/// caller reading our stdout through a pipe waits on the orphan's copy
/// of the write end long after we are gone. `timeout`, a CI cancel or
/// any SIGKILL aimed at `hale` used to leave exactly that: a hung
/// caller and a process nobody knows to kill.
///
/// Two layers, neither of which the child has to cooperate with:
///
///   * `PR_SET_PDEATHSIG` — the kernel signals the child the moment
///     the thread that forked it dies, whatever killed us, SIGKILL
///     included. The `getppid` check closes the window where we die
///     between the fork and the `prctl`, in which the setting would
///     be armed against a death that already happened. It compares
///     against OUR pid rather than testing for pid 1, so a `hale`
///     legitimately parented by an init in a container is not read
///     as an orphan.
///   * the process GROUP, which we deliberately leave alone: no
///     `setsid`, no `setpgid`, so a group-directed kill (a shell's
///     Ctrl-C, `timeout` without `--foreground`) reaches the child
///     the same way it reaches us.
///
/// Every caller waits on the child it starts on the thread that
/// started it, so the forking thread cannot exit early and retire
/// the signal under a child that should still be running.
/// The pid `hale run`'s SIGTERM handler forwards to (GH #1039).
pub(crate) static RUN_CHILD_PID: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

pub(crate) extern "C" fn forward_to_run_child(sig: libc::c_int) {
    let pid = RUN_CHILD_PID.load(std::sync::atomic::Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill(2) is async-signal-safe.
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

/// Spawn `cmd` and wait for it, standing aside for its signals (GH
/// #1039). The program drains on SIGINT / SIGTERM, so `hale run` must
/// not end first and leave it orphaned mid-drain:
///
/// * **SIGINT is ignored.** A terminal's Ctrl-C reaches the whole
///   foreground process group, the program included; `hale` just
///   keeps waiting and reports how the drain ended.
/// * **SIGTERM is forwarded.** A `kill` names `hale`'s pid only, so
///   the program would never hear it; the handler passes it on, and
///   `hale` keeps waiting.
///
/// Both dispositions change only AFTER the spawn — an ignored SIGINT
/// is inherited across exec, and the program must get the default —
/// and are restored once the program has ended.
pub(crate) fn wait_passing_signals(
    cmd: &mut std::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    let mut child = cmd.spawn()?;
    RUN_CHILD_PID.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
    // SAFETY: plain sigaction(2) calls; the handler only calls kill.
    let (old_int, old_term) = unsafe {
        let mut ign: libc::sigaction = std::mem::zeroed();
        ign.sa_sigaction = libc::SIG_IGN;
        libc::sigemptyset(&mut ign.sa_mask);
        let mut fwd: libc::sigaction = std::mem::zeroed();
        fwd.sa_sigaction = forward_to_run_child as extern "C" fn(libc::c_int) as usize;
        libc::sigemptyset(&mut fwd.sa_mask);
        fwd.sa_flags = libc::SA_RESTART;
        let mut old_int: libc::sigaction = std::mem::zeroed();
        let mut old_term: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGINT, &ign, &mut old_int);
        libc::sigaction(libc::SIGTERM, &fwd, &mut old_term);
        (old_int, old_term)
    };
    let status = child.wait();
    // SAFETY: restoring the dispositions saved above.
    unsafe {
        libc::sigaction(libc::SIGINT, &old_int, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &old_term, std::ptr::null_mut());
    }
    RUN_CHILD_PID.store(0, std::sync::atomic::Ordering::SeqCst);
    status
}

/// A private directory for what one `hale run` / `test` / `replay` /
/// `bench` compiles: the binary, the object files codegen writes
/// beside it, a replay's status and verification files. Made under
/// the temp directory with mode 0700 and a name nobody else can have
/// picked (`create_dir` fails on an existing path, symlink included,
/// and the name is retried), and removed with everything in it when
/// the guard drops — on every return, not only the one that
/// remembered to `remove_file`.
///
/// It replaces `temp_dir()/hale_run_<hash>` and its siblings: a name
/// derived from the program, in a directory every user of the box
/// can write to, is a path another process can pre-create or race.
pub(crate) struct RunScratch {
    dir: PathBuf,
}

impl RunScratch {
    pub(crate) fn new(tag: &str) -> Result<RunScratch, String> {
        use std::os::unix::fs::DirBuilderExt;
        static NONCE: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let base = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let mut last = None;
        for _ in 0..32 {
            let n = NONCE.fetch_add(1, Ordering::Relaxed);
            let dir = base.join(format!(
                "hale-{tag}-{}-{n}-{stamp:08x}",
                std::process::id()
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => return Ok(RunScratch { dir }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    last = Some(e);
                }
                Err(e) => {
                    return Err(format!(
                        "cannot make a scratch directory under {}: {e}",
                        base.display()
                    ));
                }
            }
        }
        Err(format!(
            "cannot make a scratch directory under {}: {}",
            base.display(),
            last.map(|e| e.to_string()).unwrap_or_default()
        ))
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for RunScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) fn dies_with_us(cmd: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let us = std::process::id() as libc::pid_t;
        // SAFETY: the closure runs between fork and exec in the
        // child. `prctl`, `getppid` and `_exit` are async-signal-safe
        // and allocate nothing.
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != us {
                    libc::_exit(0);
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cmd;
    }
}
