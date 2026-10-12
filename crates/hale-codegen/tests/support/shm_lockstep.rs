//! A Hale consumer of a foreign SHM ring, fed by the C driver's external
//! producer in lockstep (`shm_ring_layout_driver.c`, `step`).
//!
//! The producer used to keep to the clock: a 250 ms wait for the consumer
//! to attach, a pace per record, a 400 ms drain, with the consumer
//! started 80 ms after it and asleep for a fixed 1.2–1.5 s. A loaded
//! machine outlasts each: a consumer that attaches late misses the first
//! records, one descheduled for a few records' pace on a small ring is
//! lapped and resyncs past them (by design: a lapped reader drops what was
//! overwritten), and one whose sleep ends first misses the last. In
//! lockstep nothing waits on the clock: the producer says when the ring
//! exists, the consumer when it is subscribed (`ready`, printed once its
//! subscribers are born, which is when its reader takes its start), and
//! each next record goes out once the consumer has printed the last.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// How long one step may take: a line a process owes the test.
const STEP: Duration = Duration::from_secs(30);

/// The consumer's `main()` tail: say it is subscribed, then hold the
/// process until the test closes its stdin.
pub const CONSUMER_TAIL: &str = "println(\"ready\");\n            let _end = std::io::stdin::read_line();";

pub struct Run {
    pub producer: ExitStatus,
    pub consumer: ExitStatus,
    /// What the consumer printed after `ready`.
    pub stdout: String,
    pub stderr: String,
}

fn lines_of(r: impl Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for l in BufReader::new(r).lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    rx
}

/// Run `producer <mode> <shm> step` and `consumer`, `records` records in
/// lockstep (the consumer prints one line per record).
pub fn run(producer_bin: &Path, mode: &str, shm: &str, consumer_bin: &Path, records: usize) -> Run {
    let mut producer = Command::new(producer_bin)
        .args([mode, shm, "step"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn producer");
    let mut to_producer = producer.stdin.take().unwrap();
    let from_producer = lines_of(producer.stdout.take().unwrap());
    let ring = from_producer.recv_timeout(STEP);
    assert_eq!(ring.as_deref(), Ok("ring"), "the producer did not create its ring: {:?}", producer.try_wait());

    let mut consumer = Command::new(consumer_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn consumer");
    let to_consumer = consumer.stdin.take().unwrap();
    let from_consumer = lines_of(consumer.stdout.take().unwrap());
    let mut err = consumer.stderr.take().unwrap();
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });

    let mut seen = Vec::new();
    if from_consumer.recv_timeout(STEP).as_deref() == Ok("ready") {
        for _ in 0..records {
            // the next record; a producer that has gone is the assertion's to report
            if to_producer.write_all(b"\n").and_then(|_| to_producer.flush()).is_err() {
                break;
            }
            match from_consumer.recv_timeout(STEP) {
                Ok(l) => seen.push(l),
                Err(_) => break,
            }
        }
    }
    // the producer unlinks its ring, and the consumer's main returns
    drop(to_producer);
    let producer = producer.wait().expect("wait producer");
    drop(to_consumer);
    let consumer = consumer.wait().expect("wait consumer");
    while let Ok(l) = from_consumer.recv_timeout(Duration::from_secs(10)) {
        seen.push(l);
    }
    Run { producer, consumer, stdout: seen.join("\n"), stderr: err.join().unwrap_or_default() }
}
