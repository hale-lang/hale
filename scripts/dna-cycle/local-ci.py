#!/usr/bin/env python3
# Local CI for the file forge: for each review the host opened under <app>/.hale/dna/forge without a .checks file,
# check out its candidate in a scratch worktree, run every job of .github/workflows/*.yml (its `run:` steps; `uses:`
# steps are skipped), and write `<job name> success|failure <sha> local` per job, as a CI would report them.
import glob, os, subprocess, sys, tempfile, yaml
app = sys.argv[1]; forge = os.path.join(app, '.hale/dna/forge'); out = []
for rv in sorted(glob.glob(forge + '/*.review')):
    n = rv[:-len('.review')]
    if os.path.exists(n + '.checks'): continue
    sha = next((l.split(':', 1)[1].strip() for l in open(rv) if l.startswith('candidate:')), '')
    if not sha: continue
    wt = tempfile.mkdtemp(prefix='local-ci-')
    subprocess.run(['git', '-C', app, 'worktree', 'add', '--detach', wt, sha], capture_output=True)
    lines = []
    for wf in glob.glob(os.path.join(wt, '.github/workflows/*.yml')):
        for jid, job in (yaml.safe_load(open(wf)).get('jobs') or {}).items():
            name = job.get('name', jid); ok = True; log = ''
            for st in job.get('steps', []):
                if 'run' not in st: continue
                r = subprocess.run(['bash', '-c', st['run']], cwd=wt, capture_output=True, text=True, timeout=900)
                log += f"$ {st['run']}\n{r.stdout[-400:]}{r.stderr[-400:]}\n"
                if r.returncode != 0: ok = False; break
            lines.append(f"{name} {'success' if ok else 'failure'} {sha} local:{os.path.basename(n)}")
            out.append(f"{os.path.basename(n)} {name}: {'success' if ok else 'failure'}" + ('' if ok else ' | ' + log.strip().splitlines()[-1][:160]))
    open(n + '.checks', 'w').write('\n'.join(lines) + '\n')
    subprocess.run(['git', '-C', app, 'worktree', 'remove', '--force', wt], capture_output=True)
print('\n'.join(out) if out else 'local-ci: nothing to run')
