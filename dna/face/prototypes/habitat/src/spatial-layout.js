function computeSpatialLayout(nodes, edges, preferredRoot) {
  // A stable drawing of connectivity: the radial scaffold is not an ownership tree.
  const nodeById = new Map((nodes || []).map(node => [node.id, node]));
  const compare = (a, b) => String(a) < String(b) ? -1 : String(a) > String(b) ? 1 : 0;
  const ids = [...nodeById.keys()].sort(compare);
  const result = new Map();
  if (!ids.length) return result;
  if (ids.length === 1) return new Map([[ids[0], { x: 0, y: 0 }]]);
  const hash = value => {
    let valueHash = 2166136261;
    for (const char of String(value)) valueHash = Math.imul(valueHash ^ char.charCodeAt(0), 16777619);
    return (valueHash >>> 0) / 4294967296;
  };
  const adjacent = new Map(ids.map(id => [id, new Set()]));
  for (const edge of edges || []) {
    if (!adjacent.has(edge.from) || !adjacent.has(edge.to) || edge.from === edge.to) continue;
    adjacent.get(edge.from).add(edge.to);
    adjacent.get(edge.to).add(edge.from);
  }
  const byDegree = (a, b) => adjacent.get(b).size - adjacent.get(a).size || compare(a, b);
  const rootId = nodeById.has(preferredRoot) ? preferredRoot : [...ids].sort(byDegree)[0];
  const visited = new Set();
  const components = [];
  for (const start of [rootId, ...ids]) {
    if (visited.has(start)) continue;
    const queue = [start];
    visited.add(start);
    for (let index = 0; index < queue.length; index++) {
      for (const next of [...adjacent.get(queue[index])].sort(compare)) {
        if (!visited.has(next)) { visited.add(next); queue.push(next); }
      }
    }
    components.push(queue.sort(compare));
  }
  components.sort((a, b) => Number(b.includes(rootId)) - Number(a.includes(rootId)) || b.length - a.length || compare(a[0], b[0]));
  const layouts = components.map((component, componentIndex) => {
    const centerId = component.includes(rootId) ? rootId : [...component].sort(byDegree)[0];
    const depth = new Map([[centerId, 0]]);
    const children = new Map(component.map(id => [id, []]));
    const parent = new Map();
    const queue = [centerId];
    for (let index = 0; index < queue.length; index++) {
      const id = queue[index];
      for (const next of [...adjacent.get(id)].sort(byDegree)) {
        if (depth.has(next)) continue;
        depth.set(next, depth.get(id) + 1);
        parent.set(next, id);
        children.get(id).push(next);
        queue.push(next);
      }
    }
    const mass = new Map();
    for (const id of [...queue].reverse()) {
      mass.set(id, children.get(id).length ? children.get(id).reduce((sum, child) => sum + mass.get(child), 0) + 0.3 : 1);
    }
    const anchors = new Map([[centerId, { x: 0, y: 0 }]]);
    const orientation = hash(centerId) * Math.PI * 2;
    const place = (id, angleStart, angleEnd) => {
      const descendants = children.get(id);
      const total = descendants.reduce((sum, child) => sum + mass.get(child), 0);
      let cursor = angleStart;
      for (const child of descendants) {
        const span = (angleEnd - angleStart) * mass.get(child) / total;
        const level = depth.get(child);
        const angle = cursor + span / 2 + Math.sin(level * 0.75 + orientation) * Math.min(0.16, span * 0.07);
        const radius = level * 118 + (hash(child) - 0.5) * 14;
        anchors.set(child, { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius });
        place(child, cursor + span * 0.035, cursor + span * 0.965);
        cursor += span;
      }
    };
    place(centerId, orientation, orientation + Math.PI * 2);
    const positions = new Map(component.map(id => [id, { ...anchors.get(id), vx: 0, vy: 0 }]));
    const links = [];
    for (const from of component) {
      for (const to of [...adjacent.get(from)].sort(compare)) {
        if (compare(from, to) < 0) links.push([from, to]);
      }
    }
    for (let iteration = 0; iteration < 190; iteration++) {
      const force = new Map(component.map(id => [id, { x: 0, y: 0 }]));
      for (let left = 0; left < component.length; left++) {
        for (let right = left + 1; right < component.length; right++) {
          const a = positions.get(component[left]), b = positions.get(component[right]);
          let dx = b.x - a.x, dy = b.y - a.y;
          let distance = Math.hypot(dx, dy);
          if (distance < 0.001) {
            const angle = hash(String(component[left]) + '|' + String(component[right])) * Math.PI * 2;
            dx = Math.cos(angle); dy = Math.sin(angle); distance = 1;
          }
          const strength = Math.min(18, 4600 / (distance * distance) + Math.max(0, 80 - distance) * 0.15);
          const fx = dx / distance * strength, fy = dy / distance * strength;
          force.get(component[left]).x -= fx; force.get(component[left]).y -= fy;
          force.get(component[right]).x += fx; force.get(component[right]).y += fy;
        }
      }
      for (const [from, to] of links) {
        const a = positions.get(from), b = positions.get(to);
        const distance = Math.max(1, Math.hypot(b.x - a.x, b.y - a.y));
        const scaffoldLink = parent.get(from) === to || parent.get(to) === from;
        const strength = (distance - 112) * (scaffoldLink ? 0.038 : 0.019);
        const fx = (b.x - a.x) / distance * strength, fy = (b.y - a.y) / distance * strength;
        force.get(from).x += fx; force.get(from).y += fy;
        force.get(to).x -= fx; force.get(to).y -= fy;
      }
      for (const id of component) {
        if (id === centerId) continue;
        const point = positions.get(id), anchor = anchors.get(id), f = force.get(id);
        f.x += (anchor.x - point.x) * 0.045;
        f.y += (anchor.y - point.y) * 0.045;
        point.vx = (point.vx + f.x * 0.55) * 0.68;
        point.vy = (point.vy + f.y * 0.55) * 0.68;
        point.x += Math.max(-8, Math.min(8, point.vx));
        point.y += Math.max(-8, Math.min(8, point.vy));
      }
    }
    const radius = Math.max(1, ...[...positions.values()].map(point => Math.hypot(point.x, point.y)));
    const targetRadius = component.length === 1 ? 0 : 72 * Math.sqrt(component.length);
    for (const point of positions.values()) { point.x *= targetRadius / radius; point.y *= targetRadius / radius; }
    const packRadius = Math.max(35, targetRadius);
    const angle = componentIndex * 2.399963229728653 + orientation;
    const distance = componentIndex ? (72 * Math.sqrt(components[0].length) + packRadius + 110) * Math.sqrt(componentIndex) : 0;
    return { positions, radius: packRadius, x: Math.cos(angle) * distance, y: Math.sin(angle) * distance };
  });
  // Disconnected islands are packed separately; no synthetic connecting edges are added.
  for (let iteration = 0; iteration < 70; iteration++) {
    for (let left = 0; left < layouts.length; left++) {
      for (let right = left + 1; right < layouts.length; right++) {
        const a = layouts[left], b = layouts[right];
        const dx = b.x - a.x, dy = b.y - a.y, distance = Math.max(1, Math.hypot(dx, dy));
        const overlap = a.radius + b.radius + 65 - distance;
        if (overlap <= 0) continue;
        const shift = overlap * 0.52;
        if (left > 0) { a.x -= dx / distance * shift; a.y -= dy / distance * shift; }
        b.x += dx / distance * shift; b.y += dy / distance * shift;
      }
    }
  }
  for (const layout of layouts) {
    for (const [id, point] of layout.positions) result.set(id, { x: point.x + layout.x, y: point.y + layout.y });
  }
  const values = [...result.values()];
  const minX = Math.min(...values.map(point => point.x)), maxX = Math.max(...values.map(point => point.x));
  const minY = Math.min(...values.map(point => point.y)), maxY = Math.max(...values.map(point => point.y));
  const centerX = (minX + maxX) / 2, centerY = (minY + maxY) / 2;
  const scale = 580 / Math.max(1, maxX - minX, maxY - minY);
  for (const point of result.values()) { point.x = (point.x - centerX) * scale; point.y = (point.y - centerY) * scale; }
  return new Map(ids.map(id => [id, result.get(id)]));
}
