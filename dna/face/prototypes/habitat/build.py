#!/usr/bin/env python3
"""Build the standalone Habitat design prototype using only Python's standard library."""
from pathlib import Path
import argparse
import json
import re

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / 'src'

data = {}
for group in ('meaning', 'runtime'):
    data.update(json.loads((SOURCE / f'graph-data-{group}.json').read_text(encoding='utf-8')))
expected = {'head','face','heart','spine','nerves','memory','record','genome','body','legs','hands','hat','voice','senses','reflexes','skin'}
assert set(data) == expected
dimensions = json.loads((SOURCE / 'graph-dimensions.json').read_text(encoding='utf-8'))
dimension_keys = set()
canonical = {
    'face': ('Atlas Face', 'Interface'),
    'hat': ('Work context · t142', 'Context snapshot'),
    'record': ('Atlas record', 'Record'),
    'task-t142': ('Assess storage migration', 'Task'),
    'worker-agent1': ('Agent · slot 1', 'Worker'),
    'topic-atlas': ('Atlas events', 'Topic family'),
    'outage-api0-0931': ('api-0 outage · 09:31', 'Outage'),
    'reflex-action-0931': ('Restarted api-0', 'Action receipt'),
}
for part, page in data.items():
    for view in page['views']:
        for node in view['nodes']:
            if node['id'] in canonical:
                node['label'], node['kind'] = canonical[node['id']]
        ids = {node['id'] for node in view['nodes']}
        assert len(ids) == len(view['nodes'])
        for edge in view['edges']:
            assert edge['from'] in ids and edge['to'] in ids, (part, view['id'], edge)
            key = '::'.join((edge['from'], edge['label'], edge['to']))
            assert key in dimensions, ('Missing relationship dimension', key)
            assert dimensions[key] in {'structure', 'work', 'signal', 'evidence', 'authority'}, (key, dimensions[key])
            edge['dimension'] = dimensions[key]
            dimension_keys.add(key)
        for region in view.get('regions', []):
            assert set(region['nodeIds']) <= ids
assert dimension_keys == set(dimensions), ('Unused relationship dimensions', set(dimensions) - dimension_keys)
activity = {}
for group in ('meaning', 'runtime'):
    activity.update(json.loads((SOURCE / f'event-data-{group}.json').read_text(encoding='utf-8')))
assert set(activity) == expected
event_ids = set()
for part, page in activity.items():
    for event in page['events']:
        assert event['id'] not in event_ids, event['id']
        event_ids.add(event['id'])
        view = next(view for view in data[part]['views'] if view['id'] == event['view'])
        assert set(event['entityIds']) <= {node['id'] for node in view['nodes']}, event['id']
        assert set(event['edgeIds']) <= {'::'.join((edge['from'],edge['label'],edge['to'])) for edge in view['edges']}, event['id']
        assert event['effect'] in ('state','change','attention','flow')
        assert event['source'] in ('record','runtime','telemetry','head')
        assert event['effect'] != 'flow' or event['source'] == 'runtime'
    for view in data[part]['views']:
        assert len([event for event in page['events'] if event['view'] == view['id']]) >= 3


fragment = (SOURCE / 'shell.html').read_text(encoding='utf-8')
pages = '\n'.join((SOURCE / ('pages-' + group + '.html')).read_text(encoding='utf-8') for group in ('boundary','memory','work','runtime'))
page_ids = re.findall(r'<template id="page-([a-z]+)"', pages)
assert len(page_ids) == len(set(page_ids)) and set(page_ids) == expected
assert set(re.findall(r'data-inspect="([^"]+)"', pages)) <= set(re.findall(r'data-detail="([^"]+)"', pages))
assert set(re.findall(r'data-go-part="([^"]+)"', pages)) <= expected
css = '\n'.join((SOURCE / name).read_text(encoding='utf-8') for name in ('graph.css','spatial.css','event.css'))
engine = '  const graphData = ' + json.dumps(data,ensure_ascii=False,separators=(',', ':')) + ';\n  const activityData = ' + json.dumps(activity,ensure_ascii=False,separators=(',', ':')) + ';\n' + '\n'.join((SOURCE / name).read_text(encoding='utf-8') for name in ('graph-engine.js','spatial-layout.js','spatial-engine.js','event-engine.js'))
replacements = {
    '<!-- GRAPH_SHELL -->': (SOURCE / 'graph-shell.html').read_text(encoding='utf-8'),
    '<!-- BODY_PART_TEMPLATES -->': pages,
    '/* GRAPH_CSS */': css,
    '  // GRAPH_ENGINE': engine,
}
for marker, content in replacements.items():
    assert fragment.count(marker) == 1, marker
    fragment = fragment.replace(marker, content)
assert 'window.openai' not in fragment
wrapper = (SOURCE / 'standalone.html').read_text(encoding='utf-8')
assert wrapper.count('<!-- HABITAT_FRAGMENT -->') == 1
result = wrapper.replace('<!-- HABITAT_FRAGMENT -->', fragment)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--check',action='store_true',help='fail if index.html differs from the sources')
args = parser.parse_args()
destination = ROOT / 'index.html'
if args.check:
    if not destination.exists() or destination.read_text(encoding='utf-8') != result:
        parser.exit(1,'index.html is stale; run build.py\n')
else:
    destination.write_text(result,encoding='utf-8')
print(f'{"Verified" if args.check else "Built"} index.html: {len(data)} parts, {sum(len(page["views"]) for page in data.values())} perspectives, {len(event_ids)} events')
