#!/usr/bin/env python3
# Set how a generated organization's models answer (dna/org/models.hl), and the model its legs use
# (dna/org/work.hl).
#
#   models.hl canned <dir>     every backend is a dna::FakeModel over <dir>/rules.txt and <dir>/answers
#   models.hl record           every backend is wrapped in dna::RecordedModel (HALE_DNA_TAPE says record or replay)
#   models.hl live             unchanged
#   work.hl gateway <url>      the leg's model performer is an OpenAI-compatible chat at <url> (the key from
#                              the vault as CANNED_GATEWAY_KEY), sending metadata
import re, sys

mode, p = sys.argv[1], sys.argv[-1]
s = open(p).read()

if mode == 'gateway':
    url = sys.argv[2]
    chat = 'dna::OpenAiChat { name: "%s", endpoint: "' + url + '/v1/chat/completions", model: "canned-%s", credential: dna::HostedCredential { key: "CANNED_GATEWAY_KEY" }, send_metadata: true, input_micros_per_1k: 0, output_micros_per_1k: 0 }'
    body = 'legs::ModelPerformer { effect: "effect_free", router: dna::ModelRouter { quick: %s, deep: %s, private: %s } }' % (chat % ('quick', 'quick'), chat % ('deep', 'deep'), chat % ('private', 'private'))
    m = re.search(r'fn model\(\) -> [\w:]+ \{\n    return .*?;\n\}', s, re.S)
    if not m: sys.exit('work.hl: no model() to replace')
    s = s[:m.start()] + 'fn model() -> legs::ModelPerformer {\n    return %s;\n}' % body + s[m.end():]
    if 'import "vendor/dna" as dna;' not in s:
        s = s.replace('import "vendor/dna/legs" as legs;', 'import "vendor/dna" as dna;\nimport "vendor/dna/legs" as legs;', 1)
    open(p, 'w').write(s); print('work: the model leg asks %s' % url); sys.exit(0)

if mode == 'live': print('models: live'); sys.exit(0)
n = 0
for name in ('frontier', 'fast', 'desk'):
    m = re.search(r'fn ' + name + r'\(\) -> ([\w:]+) \{\n    return (.*?);\n\}', s, re.S)
    if not m: continue
    inner = m.group(2); label = re.search(r'name: "([^"]+)"', inner).group(1)
    if mode == 'canned':
        d = sys.argv[2]
        body = 'dna::FakeModel { name: "%s", rules_file: "%s/rules.txt", answers_dir: "%s/answers" }' % (label, d, d)
        ty = 'dna::FakeModel'
    else:
        body = 'dna::RecordedModel { name: "%s", dir: ".hale/dna/tape", dir_env: "HALE_DNA_TAPE_DIR", mode_env: "HALE_DNA_TAPE", inner: %s }' % (label, inner)
        ty = 'dna::RecordedModel'
    s = s[:m.start()] + 'fn %s() -> %s {\n    return %s;\n}' % (name, ty, body) + s[m.end():]; n += 1
open(p, 'w').write(s); print('models: %s (%d backend(s))' % (mode, n))
