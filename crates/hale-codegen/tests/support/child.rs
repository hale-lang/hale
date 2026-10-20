//! A test's child process: what it wrote to stderr, and the port it
//! listens on.
//!
//! A program a test serves binds port 0 and the test reads back the port
//! the kernel gave it ([`tcp_port`], [`udp_port`], [`bound_port`]). A
//! port a test draws (bind `:0`, read the number, close) and hands over
//! is free, not reserved: under a parallel run another test can bind it
//! before the program does. A program that must be told its address
//! before it starts takes a [`Reserved`] one; one that must find nothing
//! listening takes a [`Refusing`] one.
//!
//! Linux reads the sockets from `/proc`; elsewhere (macOS) from `lsof`.
//! A test that binds 127.0.0.2 and up is Linux's alone: macOS carries only
//! 127.0.0.1 on its loopback unless aliases are added, and such a test is
//! `ignore`d there with that reason.

#![allow(dead_code)]

#[cfg(target_os = "linux")]
use std::collections::BTreeSet;
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::process::Child;
use std::time::{Duration, Instant};

/// A child's stderr, read on a thread of its own as it is written, so a
/// failure says what the program said.
#[derive(Clone, Default)]
pub struct Stderr {
    text: std::sync::Arc<std::sync::Mutex<String>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Stderr {
    pub fn of(child: &mut Child) -> Stderr {
        let s = Stderr::default();
        if let Some(mut e) = child.stderr.take() {
            let t = s.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n @ 1..) = e.read(&mut buf) {
                    t.text
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                t.done.store(true, std::sync::atomic::Ordering::SeqCst);
            });
        }
        s
    }

    pub fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    /// All of it, once the child has exited (bounded: a grandchild could
    /// hold the pipe).
    pub fn finished(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.done.load(std::sync::atomic::Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.text()
    }
}

/// The port the process listens on: its one listening TCP socket. The
/// programs here bind port 0 and the kernel picks. Read once the program
/// has said it is up, so the socket is there ([`tcp_port`] waits for it).
pub fn bound_port(pid: u32, err: &Stderr) -> u16 {
    let ports = listening_ports(pid);
    assert_eq!(
        ports.len(),
        1,
        "one listening socket: {ports:?}; stderr:\n{}",
        err.text()
    );
    ports[0]
}

/// The local ports of the process's listening TCP sockets.
pub fn listening_ports(pid: u32) -> Vec<u16> {
    listening_addrs(pid).iter().map(|a| a.port()).collect()
}

/// The local addresses of the process's listening TCP (IPv4) sockets. A
/// program with several listeners binds each on a loopback address of its
/// own (`127.0.0.2:0`, …), so the port the kernel gave each is told apart
/// by the address.
///
/// A socket this process holds too is not the child's: a child that has
/// not finished its exec still holds the test's descriptors (the test's
/// own listener, say, created before the spawn), and `spawn` can return
/// while the kernel is still closing them.
#[cfg(target_os = "linux")]
pub fn listening_addrs(pid: u32) -> Vec<SocketAddrV4> {
    sockets_of(pid, "tcp")
        .into_iter()
        .filter(|s| s.state == "0A")
        .map(|s| s.local)
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub fn listening_addrs(pid: u32) -> Vec<SocketAddrV4> {
    lsof_addrs(pid, &["-i4TCP", "-sTCP:LISTEN"])
}

/// The local addresses of the process's bound, unconnected UDP (IPv4)
/// sockets: the ones that receive what is sent to them.
#[cfg(not(target_os = "linux"))]
pub fn udp_addrs(pid: u32) -> Vec<SocketAddrV4> {
    lsof_addrs(pid, &["-i4UDP"])
}

/// The local addresses `lsof` lists for process `pid` under `select`,
/// less those this process holds too (see [`listening_addrs`]). Its `-Fn`
/// field is `n<local>` (`127.0.0.1:5000`, `*:5000` for the wildcard) or
/// `n<local>-><remote>` for a connected socket, which is not one that
/// listens or receives from anyone.
#[cfg(not(target_os = "linux"))]
fn lsof_addrs(pid: u32, select: &[&str]) -> Vec<SocketAddrV4> {
    let of = |pid: u32| -> Vec<SocketAddrV4> {
        let Ok(out) = std::process::Command::new("lsof")
            .args(["-nP", "-a", "-p", &pid.to_string()])
            .args(select)
            .arg("-Fn")
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix('n'))
            .filter(|n| !n.contains("->"))
            .filter_map(|n| match n.strip_prefix("*:") {
                Some(port) => port
                    .parse()
                    .ok()
                    .map(|p| SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, p)),
                None => n.parse().ok(),
            })
            .collect()
    };
    let ours = of(std::process::id());
    of(pid).into_iter().filter(|a| !ours.contains(a)).collect()
}

#[cfg(target_os = "linux")]
pub fn udp_addrs(pid: u32) -> Vec<SocketAddrV4> {
    sockets_of(pid, "udp")
        .into_iter()
        .filter(|s| s.local.port() != 0 && s.remote.port() == 0)
        .map(|s| s.local)
        .collect()
}

/// Whether process `pid` has exited: gone, or a zombie not yet reaped
/// (which holds no descriptors). Linux reads `/proc/<pid>/stat`;
/// elsewhere `ps -o stat=` (empty once it is gone, `Z…` for a zombie).
pub fn exited(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(") ")
                .and_then(|(_, rest)| rest.chars().next())
        });
    #[cfg(not(target_os = "linux"))]
    let state = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().chars().next());
    matches!(state, None | Some('Z') | Some('X'))
}

/// The port `child`'s one listening TCP socket was given, once it listens.
/// A child that exits first fails the test with what it printed.
pub fn tcp_port(child: &mut Child) -> u16 {
    tcp_port_within(child, Duration::from_secs(60))
}

/// [`tcp_port`], for a child that may first build what it serves (a
/// toolchain cache that is cold on a fresh box).
pub fn tcp_port_within(child: &mut Child, limit: Duration) -> u16 {
    wait_for(child, "a listening TCP socket", limit, |pid| {
        listening_addrs(pid)
    })
    .port()
}

/// The port `child`'s one listening TCP socket on `host` was given (a
/// program with several listeners binds each on a loopback address of
/// its own: `127.0.0.1:0`, `127.0.0.2:0`, …).
pub fn tcp_port_on(child: &mut Child, host: Ipv4Addr) -> u16 {
    wait_for(
        child,
        &format!("a listening TCP socket on {host}"),
        Duration::from_secs(60),
        |pid| {
            listening_addrs(pid)
                .into_iter()
                .filter(|a| *a.ip() == host)
                .collect()
        },
    )
    .port()
}

/// The port `child`'s one bound, unconnected UDP socket was given, once
/// it has one.
pub fn udp_port(child: &mut Child) -> u16 {
    wait_for(
        child,
        "a bound UDP socket",
        Duration::from_secs(60),
        |pid| udp_addrs(pid),
    )
    .port()
}

/// The port `child`'s one bound, unconnected UDP socket on `host` was
/// given.
pub fn udp_port_on(child: &mut Child, host: Ipv4Addr) -> u16 {
    wait_for(
        child,
        &format!("a bound UDP socket on {host}"),
        Duration::from_secs(60),
        |pid| {
            udp_addrs(pid)
                .into_iter()
                .filter(|a| *a.ip() == host)
                .collect()
        },
    )
    .port()
}

/// Poll `probe` until it finds exactly one socket. The child exiting
/// first, or two sockets where one was meant, fails the test.
fn wait_for(
    child: &mut Child,
    what: &str,
    limit: Duration,
    probe: impl Fn(u32) -> Vec<SocketAddrV4>,
) -> SocketAddrV4 {
    let start = Instant::now();
    loop {
        let found = probe(child.id());
        assert!(found.len() <= 1, "{what}, not several: {found:?}");
        if let Some(a) = found.first() {
            return *a;
        }
        if let Ok(Some(status)) = child.try_wait() {
            let mut out = String::new();
            let mut err = String::new();
            if let Some(mut o) = child.stdout.take() {
                let _ = o.read_to_string(&mut out);
            }
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            panic!("the program exited ({status}) before it had {what}\nstdout:\n{out}\nstderr:\n{err}");
        }
        assert!(start.elapsed() < limit, "no {what} within {limit:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The loopback address a [`Reserved`] port is handed out on. Nothing
/// else in the suite binds it.
const RESERVED_HOST: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 100);

/// Where [`Reserved`] addresses are: [`RESERVED_HOST`] where the
/// loopback carries it (Linux routes all of 127/8 there), else
/// `127.0.0.1` (macOS, which carries only that one by default).
pub fn reserved_host() -> Ipv4Addr {
    static HOST: std::sync::OnceLock<Ipv4Addr> = std::sync::OnceLock::new();
    *HOST.get_or_init(|| {
        if UdpSocket::bind((RESERVED_HOST, 0)).is_ok() {
            RESERVED_HOST
        } else {
            Ipv4Addr::LOCALHOST
        }
    })
}

/// An address a program can be told before it starts and bind once it
/// does, which no other test of the run can take in between: the test
/// holds `127.0.0.1:<port>` (TCP and UDP, kernel-picked) for as long as
/// this lives, and hands out [`RESERVED_HOST`]`:<port>`. A bind of the
/// same port on any address but `127.0.0.1` itself does not conflict with
/// the held one, so the program's bind succeeds; a wildcard bind
/// (`0.0.0.0:<port>`) does conflict, so no other test's listener can
/// cover the address; and nothing else binds [`RESERVED_HOST`].
///
/// Where the loopback does not carry [`RESERVED_HOST`] (macOS), nothing
/// can be held that leaves the port bindable on the one loopback address,
/// so the port is drawn and handed out on `127.0.0.1` unheld: free, not
/// reserved, as before this helper. Only such a host keeps that race.
///
/// For a program that must know its peer's address before the peer is up
/// (a client that dials a listener started later, a listener the program
/// opens twice). A program that only serves binds port 0 instead and the
/// test reads the port back ([`tcp_port`]).
pub struct Reserved {
    _held: Option<(TcpListener, UdpSocket)>,
    pub addr: SocketAddrV4,
}

impl Reserved {
    pub fn new() -> Reserved {
        // the same port held for both protocols, so either is reserved
        for _ in 0..64 {
            let tcp = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
            let port = tcp.local_addr().expect("the held address").port();
            if let Ok(udp) = UdpSocket::bind(("127.0.0.1", port)) {
                let host = reserved_host();
                // on 127.0.0.1 itself the held sockets would be the program's bind's conflict
                let held = (host != Ipv4Addr::LOCALHOST).then_some((tcp, udp));
                return Reserved {
                    _held: held,
                    addr: SocketAddrV4::new(host, port),
                };
            }
        }
        panic!("no port free for both TCP and UDP in 64 tries");
    }

    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

/// An address nothing listens on, and nothing can while this lives: a
/// connect to it is refused. It is a [`Reserved`] address the test never
/// hands to anything that binds.
pub struct Refusing(Reserved);

impl Refusing {
    pub fn new() -> Refusing {
        Refusing(Reserved::new())
    }

    pub fn addr(&self) -> SocketAddrV4 {
        self.0.addr
    }

    pub fn host(&self) -> String {
        self.0.host()
    }

    pub fn port(&self) -> u16 {
        self.0.port()
    }
}

#[cfg(target_os = "linux")]
struct Socket {
    local: SocketAddrV4,
    remote: SocketAddrV4,
    state: String,
}

/// The process's sockets of `proto` (`tcp`, `udp`) from its network
/// namespace's table, those whose descriptors it holds.
///
/// A socket this process holds too is not the child's: a child that has
/// not finished its exec still holds the test's descriptors (the test's
/// own listener, say, created before the spawn), and `spawn` can return
/// while the kernel is still closing them.
#[cfg(target_os = "linux")]
fn sockets_of(pid: u32, proto: &str) -> Vec<Socket> {
    let sockets = |pid: &str| -> BTreeSet<String> {
        std::fs::read_dir(format!("/proc/{pid}/fd"))
            .map(|d| {
                d.flatten()
                    .filter_map(|e| std::fs::read_link(e.path()).ok())
                    .filter_map(|t| {
                        t.to_string_lossy()
                            .strip_prefix("socket:[")
                            .and_then(|r| r.strip_suffix(']'))
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let ours = sockets("self");
    let inodes: BTreeSet<String> = sockets(&pid.to_string())
        .difference(&ours)
        .cloned()
        .collect();
    // `0100007F:1F90`: the address's bytes in network order, printed as
    // a host-order word, then the port
    let addr = |a: &str| -> Option<SocketAddrV4> {
        let (ip, port) = a.split_once(':')?;
        let ip = u32::from_str_radix(ip, 16).ok()?;
        let port = u16::from_str_radix(port, 16).ok()?;
        Some(SocketAddrV4::new(Ipv4Addr::from(u32::from_be(ip)), port))
    };
    let table = std::fs::read_to_string(format!("/proc/{pid}/net/{proto}")).unwrap_or_default();
    table
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() > 9 && inodes.contains(f[9]) {
                Some(Socket {
                    local: addr(f[1])?,
                    remote: addr(f[2])?,
                    state: f[3].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}
