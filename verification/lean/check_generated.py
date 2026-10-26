#!/usr/bin/env python3
"""Run the real compiler producer, then kernel-check its concrete statements.

This runner contains no Hale graph extraction or claim evaluator. Rejections,
nonzero exits and timeouts fail the gate. Generated artifacts stay under target.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
PROJECT = ROOT / "verification" / "lean"


def run(command, cwd=ROOT, env=None, succeeds=True):
    result = subprocess.run([str(arg) for arg in command], cwd=str(cwd), env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            universal_newlines=True, timeout=180)
    if (result.returncode == 0) != succeeds:
        raise SystemExit("unexpected exit {}: {}\n{}".format(result.returncode, command, result.stdout))
    return result.stdout


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lake", default="lake")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    version = run([args.lake, "env", "lean", "--version"], PROJECT)
    if "version 4.35.0," not in version:
        raise SystemExit("pinned Lean 4.35.0 required: " + version)
    print(version.strip(), flush=True)
    command = ["cargo", "build", "--release", "-p", "hale-types", "--example", "reachability_proofs"]
    if args.offline:
        command.append("--offline")
    run(command)
    target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target")))
    if not target.is_absolute():
        target = ROOT / target
    exporter = target / "release" / "examples" / "reachability_proofs"
    output = target / "verification" / "lean"
    output.mkdir(parents=True, exist_ok=True)
    run([args.lake, "build"], PROJECT)
    env = dict(os.environ)
    env["LEAN_PATH"] = str(output) + os.pathsep + env.get("LEAN_PATH", "")
    expected = {"closed": "closed", "interior": "path", "reflexive": "path", "empty": "no_targets", "phase": "closed", "bus": "path", "imported": "closed", "initialize": "path", "initialize_guard": "closed", "field_initialize": "path", "field_initialize_guard": "closed", "argument_default": "path", "argument_default_guard": "closed", "closure_check": "path", "closure_check_guard": "closed", "phase_assertion": "path", "birth_check": "path", "lifecycle_birth": "path"}
    for stage in ["run", "drain", "dissolve"]:
        expected["lifecycle_" + stage] = "path"

    expected.update({"parent_accept": "path", "parent_release": "path"})
    source_cases = ["automatic_epoch", "phase_assertion_unknown", "birth_check_unknown", "parent_unknown"]
    expected.update({"source_" + case: "closed" for case in source_cases})
    expected.update({"lifecycle_quiet_" + stage: "closed" for stage in ["birth", "run", "drain", "dissolve"]})

    def source_seed(case, with_handler=False):
        # Preserve the original source fixture, adding an unreachable real
        # target so an absence theorem cannot pass through empty selection.
        seed = output / "sources" / (("handler_" if with_handler else "source_") + case)
        seed.mkdir(parents=True, exist_ok=True)
        source = (PROJECT / "fixtures" / case / "main.hl").read_text()
        source += '\nfn unrelated() { println("outside"); }\n'
        if with_handler:
            source += '\nlocus UnusedSupervisor { on_failure(c: App, e: ClosureViolation) { } }\n'
        (seed / "main.hl").write_text(source)
        return seed

    def lifecycle_seed(stage, unknown=False, quiet=False):
        # Vary source bodies; the compiler still derives every graph and claim.
        case = "lifecycle_unknown" if unknown or quiet else "lifecycle_birth"
        seed = output / "sources" / (("opaque_" if unknown else "quiet_" if quiet else "") + case + "_" + stage)
        seed.mkdir(parents=True, exist_ok=True)
        source = (PROJECT / "fixtures" / case / "main.hl").read_text()
        if source.count("birth()") != 1:
            raise SystemExit("lifecycle fixture lost its unique self hook")
        if unknown:
            if source.count("let value = 1;") != 1:
                raise SystemExit("lifecycle unknown control lost its unique body")
            source = source.replace("let value = 1;", "(match 0 { _ -> B { }, }).work();")
            source += "\nlocus B { fn work() { } }\n"
        source += '\nfn unrelated() { println("outside"); }\n'
        (seed / "main.hl").write_text(source.replace("birth()", stage + "()"))
        return seed

    for case, kind in expected.items():
        module = case.capitalize()
        report = output / (module + ".json")
        lean = output / (module + ".lean")
        for previous in [report, lean, output / (module + ".olean")]:
            if previous.exists():
                previous.unlink()
        if case.startswith("source_"):
            seed = source_seed(case[len("source_"):])
        elif case.startswith("lifecycle_quiet_"):
            seed = lifecycle_seed(case[len("lifecycle_quiet_"):], quiet=True)
        elif case.startswith("lifecycle_"):
            seed = lifecycle_seed(case[len("lifecycle_"):])
        else:
            seed = PROJECT / "fixtures" / case
        run([exporter, seed, report, lean])
        evidence = json.loads(report.read_text())
        if evidence["schema"] != "hale.reachability-proofs/2":
            raise SystemExit("unsupported evidence schema")
        declared_digest = evidence["binding"].pop("evidence_digest")
        canonical = json.dumps(evidence, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        digest = 0xcbf29ce484222325
        for byte in canonical:
            digest = ((digest ^ byte) * 0x100000001b3) & 0xffffffffffffffff
        if declared_digest != "{:016x}".format(digest):
            raise SystemExit("evidence payload digest does not bind its canonical facts")
        evidence["binding"]["evidence_digest"] = declared_digest
        row = evidence["rows"][0]
        if not evidence["all_selected_forbid_laws_have_certificates"] or not evidence["rows"] or any(r["proof"]["certificate"]["kind"] != kind for r in evidence["rows"]):
            raise SystemExit("wrong compiler evidence for " + case)
        if case == "interior" and not any(v["identity"].startswith("stdlib:") for v in row["proof"]["vertices"]):
            raise SystemExit("interior fixture lost its stdlib absorption")
        if case.startswith("initialize") and not any(v["display"] == "Settings::params::n" for v in row["proof"]["vertices"]):
            raise SystemExit("initializer fixture lost its executable default body")
        if case.startswith("field_initialize") and not any(v["display"] == "Settings::fields::n" for v in row["proof"]["vertices"]):
            raise SystemExit("field initializer fixture lost its type-owned default body")
        if case.startswith("argument_default") and not any(v["display"] == "calculate::args::n" for v in row["proof"]["vertices"]):
            raise SystemExit("argument fixture lost its callable-owned default body")
        if case.startswith("closure_check") and not any(v["display"] == "Child::closures::audit" for v in row["proof"]["vertices"]):
            raise SystemExit("closure fixture lost its executable assertion body")
        if case == "phase_assertion" and not any(v["display"] == "Worker::closures::audit" for v in row["proof"]["vertices"]):
            raise SystemExit("phase fixture lost its automatic assertion root")
        if case == "birth_check" and not any(v["display"] == "Worker::birth_checks::0" for v in row["proof"]["vertices"]):
            raise SystemExit("birth-check fixture lost its predicate root")
        if case.startswith("lifecycle_"):
            stage = case[len("lifecycle_quiet_"):] if case.startswith("lifecycle_quiet_") else case[len("lifecycle_"):]
            if not any(v["display"] == "Child::" + stage for v in row["proof"]["vertices"]):
                raise SystemExit("lifecycle fixture lost its actual " + stage + " body")
        if case in ["parent_accept", "parent_release"]:
            stage = case[len("parent_"):]
            if not any(v["display"] == "Parent::" + stage for v in row["proof"]["vertices"]):
                raise SystemExit("parent fixture lost its actual " + stage + " body")
        if case.startswith("source_") or case.startswith("lifecycle_quiet_"):
            if not row["proof"]["targets"]:
                raise SystemExit("source absence fixture lost its real effect target")
        source_body = {
            "source_automatic_epoch": "Worker::closures::audit",
            "source_phase_assertion_unknown": "Worker::closures::audit",
            "source_birth_check_unknown": "Worker::birth_checks::0",
            "source_parent_unknown": "Parent::accept",
        }.get(case)
        if source_body and not any(v["display"] == source_body for v in row["proof"]["vertices"]):
            raise SystemExit("source absence fixture lost its represented body: " + source_body)
        if case == "imported":
            origins = [r["origin"] for r in evidence["rows"]]
            if len(origins) != 2 or {"kind": "library", "alias": "lib"} not in origins or {"kind": "constitution", "name": "Boundary"} not in origins:
                raise SystemExit("import fixture lost a library/constitution law: " + repr(origins))
        if case == "reflexive" and row["proof"]["certificate"]["path"]:
            raise SystemExit("reflexive fixture did not produce a zero-length path")
        run([args.lake, "env", "lean", "-R", output, "-o", output / (module + ".olean"), lean], PROJECT, env)
        run([args.lake, "env", "leanchecker", module], PROJECT, env)
        print("{}: compiler {} certificate, Lean and leanchecker passed".format(case, kind), flush=True)
    # The kernel must reject a false closed-set certificate from an otherwise
    # authentic generated proposition. Merely having compiled the checker is not
    # enough: this exercises the instantiated certificate calculation.
    bad = re.sub(r"def closed : List Nat := .*", "def closed : List Nat := []",
                 (output / "Closed.lean").read_text())
    if bad == (output / "Closed.lean").read_text():
        raise SystemExit("negative control did not alter a certificate")
    broken = output / "Broken.lean"
    broken.write_text(bad)
    failure = run([args.lake, "env", "lean", "-R", output, broken], PROJECT, env, succeeds=False)
    if "error:" not in failure or "decide" not in failure:
        raise SystemExit("negative control failed for an unexpected reason: " + failure)
    print("omitted-root negative control: Lean rejected", flush=True)
    invalid_report = output / "Invalid.json"
    invalid_lean = output / "Invalid.lean"
    # An old successful run must never satisfy the negative assertion.
    for path in [invalid_report, invalid_lean]:
        if path.exists():
            path.unlink()
    failure = run([exporter, PROJECT / "fixtures" / "invalid", invalid_report, invalid_lean], succeeds=False)
    if "cannot emit Lean proofs" not in failure or invalid_report.exists() or invalid_lean.exists():
        raise SystemExit("invalid-law export did not fail closed: " + failure)
    print("invalid-law negative control: compiler refused theorem", flush=True)
    pending_report = output / "AutomaticEpoch.json"
    pending_lean = output / "AutomaticEpoch.lean"
    for path in [pending_report, pending_lean]:
        if path.exists():
            path.unlink()
    failure = run([exporter, source_seed("automatic_epoch", with_handler=True), pending_report, pending_lean], succeeds=False)
    if "cannot emit Lean proofs" not in failure or pending_report.exists() or pending_lean.exists():
        raise SystemExit("unresolved automatic epoch did not refuse a theorem: " + failure)
    print("automatic-epoch negative control: compiler refused theorem", flush=True)
    unknown_report = output / "UnknownPhaseAssertion.json"
    unknown_lean = output / "UnknownPhaseAssertion.lean"
    for path in [unknown_report, unknown_lean]:
        if path.exists():
            path.unlink()
    failure = run([exporter, source_seed("phase_assertion_unknown", with_handler=True), unknown_report, unknown_lean], succeeds=False)
    if "cannot emit Lean proofs" not in failure or unknown_report.exists() or unknown_lean.exists():
        raise SystemExit("unresolved phase assertion did not refuse a theorem: " + failure)
    print("phase-assertion negative control: compiler refused theorem", flush=True)
    unknown_report = output / "UnknownBirthCheck.json"
    unknown_lean = output / "UnknownBirthCheck.lean"
    for path in [unknown_report, unknown_lean]:
        if path.exists():
            path.unlink()
    failure = run([exporter, source_seed("birth_check_unknown", with_handler=True), unknown_report, unknown_lean], succeeds=False)
    if "cannot emit Lean proofs" not in failure or unknown_report.exists() or unknown_lean.exists():
        raise SystemExit("unresolved birth rejection did not refuse a theorem: " + failure)
    print("birth-check negative control: compiler refused theorem", flush=True)
    parent_report = output / "ParentUnknown.json"
    parent_lean = output / "ParentUnknown.lean"
    for path in [parent_report, parent_lean]:
        if path.exists():
            path.unlink()
    failure = run([exporter, source_seed("parent_unknown", with_handler=True), parent_report, parent_lean], succeeds=False)
    if "cannot emit Lean proofs" not in failure or parent_report.exists() or parent_lean.exists():
        raise SystemExit("unresolved parent invocation did not refuse a theorem: " + failure)
    print("parent-context negative control: compiler refused theorem", flush=True)
    for stage in ["birth", "run", "drain", "dissolve"]:
        unknown_report = output / ("LifecycleUnknown_" + stage + ".json")
        unknown_lean = output / ("LifecycleUnknown_" + stage + ".lean")
        for path in [unknown_report, unknown_lean]:
            if path.exists():
                path.unlink()
        failure = run([exporter, lifecycle_seed(stage, unknown=True), unknown_report, unknown_lean], succeeds=False)
        if "cannot emit Lean proofs" not in failure or unknown_report.exists() or unknown_lean.exists():
            raise SystemExit("opaque " + stage + " source did not refuse a theorem: " + failure)
        print("lifecycle-" + stage + "-opaque-source negative control: compiler refused theorem", flush=True)





if __name__ == "__main__":
    main()
