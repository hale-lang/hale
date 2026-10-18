#!/usr/bin/env python3
"""Hold a candidate `hale` to a base `hale`: every `hale check` output alike.

    scripts/check-cost/compare.py <base hale> <candidate hale>

Runs both binaries over every corpus example, `dna/host`, `dna/core`,
`dna/api`, the API contract program, every `tests/hale` program and a
trivial seed, each under every flag set below, and reports each run whose
stdout, stderr or exit status differs. The build-identity digests
(`inputs_digest`, `artifact_digest`, `exec_digest`) are masked: two builds
of one source differ there by construction. Exits 1 when a run differs.
"""
import glob
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
FLAGSETS = [
    [],
    ['--json'],
    ['--dump-alloc-summary'],
    ['--dump-effects-manifest'],
    ['--dump-topology'],
    ['--dump-resource-budget'],
    ['--warn-resource-leak', '--report-unbounded-use'],
    ['--api'],
]
MASK = re.compile(r'"(inputs_digest|artifact_digest|exec_digest)": "[0-9a-f]+"')
TRIVIAL = 'fn main() {\n    println("hi");\n}\n'


def targets(trivial):
    out = sorted(glob.glob(os.path.join(ROOT, 'crates/hale-codegen/tests/fixtures/examples/*/')))
    out += [os.path.join(ROOT, p) for p in ('dna/host', 'dna/core', 'dna/api', 'tests/api-contract/program.hl')]
    out += [trivial]
    out += sorted(glob.glob(os.path.join(ROOT, 'tests/hale/*_test.hl')))
    out += sorted(glob.glob(os.path.join(ROOT, 'tests/hale/api/*_test.hl')))
    return out


def run(hale, flags, target, env):
    p = subprocess.run([hale, 'check'] + flags + [target], capture_output=True, text=True, cwd=tempfile.gettempdir(), env=env, timeout=600)
    return MASK.sub(r'"\1": X', p.stdout + '\n--stderr--\n' + p.stderr) + f'\nexit={p.returncode}'


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    base, cand = (os.path.abspath(a) for a in sys.argv[1:])
    env = dict(os.environ, HALE_SKIP_STALE_CHECK='1')
    with tempfile.TemporaryDirectory() as d:
        trivial = os.path.join(d, 'main.hl')
        with open(trivial, 'w') as f:
            f.write(TRIVIAL)
        n = bad = 0
        for t in targets(trivial):
            for flags in FLAGSETS:
                n += 1
                if run(base, flags, t, env) != run(cand, flags, t, env):
                    bad += 1
                    print('DIFF:', ' '.join(flags) or '(no flags)', os.path.relpath(t, ROOT) if t.startswith(ROOT) else 'trivial seed')
    print(f'compared {n} runs, {bad} differ')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
