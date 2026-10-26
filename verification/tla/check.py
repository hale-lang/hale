#!/usr/bin/env python3
"""Run the pinned TLC wake pilot and require its negative controls to fail."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TLC_SHA256 = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
# These are source-alignment guards, not correctness proofs. Changes require
# reviewing the corresponding abstraction in WakeHandshake.tla and README.md.
REGIONS = [
    ("void lotus_mailbox_post(", "static int lotus_mailbox_take(",
     "b29fc2305100efdad8e4bd2b74a271e36e5cf5df8fc63b69518608970aeebe02"),
    ("int lotus_mailbox_drain_one(", "void lotus_mailbox_shutdown(",
     "dc2a699393ecc08b0a442bf72690a2421ddd4a9ed8445423a4f6e046a15c38c2"),
    ("void lotus_mailbox_shutdown(", "void lotus_mailbox_destroy(",
     "9a941f292b5c63668eec0bdb9378a1ea7c0cb8d40c80a4ee7d347e3abfa8c91a"),
    ("static void lotus_coop_pool_wake_consumer(", "static void lotus_coop_pool_post_cell(",
     "446512ffd9e149caa7ca22f59f058d4ed2a48f7cc1906fafad9e12432b101217"),
    ("static int lotus_coop_pool_drain_one(", "/* F.35 Slice 1: thread-locals",
     "49c2f47de6eb234021576e7621ba722a984f0f19efd637e198809fbaaa6fba40"),
]
CASES = [
    ("WakeHandshake.cfg", False),
    ("WakeShutdown.cfg", False),
    ("WakeNoRecheck.cfg", True),
    ("WakeNoSignal.cfg", True),
    ("WakeUnprotectedSignal.cfg", True),
]


def check_source():
    source = (ROOT / "crates/hale-codegen/runtime/lotus_arena.c").read_bytes()
    for start, end, expected in REGIONS:
        first = source.index(start.encode())
        last = source.index(end.encode(), first)
        actual = hashlib.sha256(source[first:last]).hexdigest()
        if actual != expected:
            raise SystemExit(
                f"wake model source drift at {start}: {actual}; review the "
                "TLA+ correspondence and exclusions before updating the guard"
            )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", default=os.environ.get("TLA2TOOLS_JAR"))
    parser.add_argument("--java", default=os.environ.get("JAVA", "java"))
    parser.add_argument("--check-source", action="store_true")
    parser.add_argument("--output", type=Path, default=ROOT / "target/verification/tla")
    args = parser.parse_args()
    check_source()
    if args.check_source:
        print("wake source correspondence guards passed")
        return
    if not args.jar:
        parser.error("provide --jar or TLA2TOOLS_JAR (see verification/tla/README.md)")
    jar = Path(args.jar).resolve()
    if hashlib.sha256(jar.read_bytes()).hexdigest() != TLC_SHA256:
        parser.error("TLC jar does not match the pinned official v1.7.4 artifact")
    subprocess.run([args.java, "-version"], check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    for config, negative in CASES:
        with tempfile.TemporaryDirectory(prefix="hale-tlc-") as scratch:
            work = Path(scratch)
            shutil.copyfile(HERE / "WakeHandshake.tla", work / "WakeHandshake.tla")
            shutil.copyfile(HERE / config, work / config)
            command = [args.java, "-XX:+UseParallelGC", "-Xmx1g", "-cp", str(jar),
                       "tlc2.TLC", "-workers", "1", "-seed", "1", "-fp", "0",
                       "-metadir", str(work / "states"), "-config", config,
                       "WakeHandshake.tla"]
            result = subprocess.run(command, cwd=work, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, universal_newlines=True, timeout=60)
            log = args.output / f"{Path(config).stem}.log"
            log.write_text(result.stdout)
            if negative:
                passed = result.returncode != 0 and "Temporal properties were violated" in result.stdout
            else:
                passed = result.returncode == 0 and "Model checking completed. No error has been found." in result.stdout
            if not passed:
                raise SystemExit(f"unexpected TLC result for {config}; see {log}\n{result.stdout[-3000:]}")
            counts = re.findall(r"\d+ states generated, \d+ distinct states found.*", result.stdout)
            print(f"PASS {config}: {'liveness counterexample required and found' if negative else 'exhaustive finite-model check'}")
            if counts:
                print(f"  {counts[-1]}")
    print(f"TLC pilot passed; full outputs and negative traces: {args.output}")


if __name__ == "__main__":
    main()
