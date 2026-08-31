const FAMILY_COLORS = {
  chat: [0.71, 0.96, 0.76, 1],
  action: [0.47, 0.71, 1, 1],
  extension: [1, 0.8, 0.46, 1],
  system: [0.85, 0.65, 1, 1],
};

export function eventFamily(event) {
  if (event.event_type.startsWith("chat.")) return "chat";
  if (event.event_type.startsWith("action.") || event.event_type === "actions.completed") return "action";
  if (event.source.startsWith("extension:") || event.source.startsWith("tool:")) return "extension";
  return "system";
}

export function memoryLayout(events) {
  const ordered = [...events].sort((left, right) => left.sequence - right.sequence || left.id.localeCompare(right.id));
  const positions = new Map();
  ordered.forEach(event => {
    const correlationHash = stableHash(event.correlation_id);
    const eventHash = stableHash(event.id);
    const angle = (correlationHash % 6283) / 1000;
    const ring = 0.42 + ((correlationHash >>> 8) % 1000) / 1000 * 0.92;
    const jitterAngle = (eventHash % 6283) / 1000;
    const jitterRadius = 0.06 + ((eventHash >>> 7) % 1000) / 1000 * 0.18;
    positions.set(event.id, {
      x: Math.cos(angle) * ring + Math.cos(jitterAngle) * jitterRadius,
      y: Math.log1p(Math.max(0, event.sequence)) * 0.35 - 1.2,
      z: Math.sin(angle) * ring + Math.sin(jitterAngle) * jitterRadius,
    });
  });
  return { ordered, positions };
}

export function causalFamily(events, selectedId) {
  const byId = new Map(events.map(event => [event.id, event]));
  const children = new Map();
  for (const event of events) {
    if (!event.causation_id) continue;
    const values = children.get(event.causation_id) || [];
    values.push(event.id);
    children.set(event.causation_id, values);
  }
  const ancestors = new Set();
  const descendants = new Set();
  let omittedAncestors = 0;
  let cursor = byId.get(selectedId);
  while (cursor?.causation_id) {
    if (!byId.has(cursor.causation_id)) {
      omittedAncestors += 1;
      break;
    }
    if (ancestors.has(cursor.causation_id) || cursor.causation_id === selectedId) break;
    ancestors.add(cursor.causation_id);
    cursor = byId.get(cursor.causation_id);
  }
  const queue = [...(children.get(selectedId) || [])];
  while (queue.length) {
    const id = queue.shift();
    if (!id || descendants.has(id) || id === selectedId) continue;
    descendants.add(id);
    queue.push(...(children.get(id) || []));
  }
  return { ancestors, descendants, omittedAncestors };
}

export function buildMemoryScene(events, links, { selectedId = null, hoveredId = null, liveIds = new Set(), now = Date.now() } = {}) {
  const { ordered, positions } = memoryLayout(events);
  const indexById = new Map(ordered.map((event, index) => [event.id, index]));
  const selected = ordered.find(event => event.id === selectedId) || null;
  const family = selected ? causalFamily(ordered, selected.id) : { ancestors: new Set(), descendants: new Set(), omittedAncestors: 0 };
  const neighbors = new Set(hoveredId ? [hoveredId] : []);
  const edges = [];
  const missingParents = [];
  let boundarySemantic = 0;
  for (const event of ordered) {
    if (!event.causation_id) continue;
    const source = indexById.get(event.causation_id);
    const target = indexById.get(event.id);
    if (source === undefined) {
      missingParents.push({ event, target });
      continue;
    }
    if (source === target) continue;
    if (hoveredId === event.id || hoveredId === event.causation_id) {
      neighbors.add(event.id);
      neighbors.add(event.causation_id);
    }
    const highlighted = selectedId === event.id || family.ancestors.has(event.id) || family.descendants.has(event.id);
    edges.push({ source, target, kind: 0, highlighted, hovered: hoveredId === event.id || hoveredId === event.causation_id, bidirectional: false });
  }
  for (const link of links) {
    const source = indexById.get(link.from_event_id);
    const target = indexById.get(link.to_event_id);
    if (source === undefined || target === undefined) {
      boundarySemantic += 1;
      continue;
    }
    if (source === target) continue;
    if (hoveredId === link.from_event_id || hoveredId === link.to_event_id) {
      neighbors.add(link.from_event_id);
      neighbors.add(link.to_event_id);
    }
    edges.push({ source, target, kind: 1, highlighted: false, hovered: hoveredId === link.from_event_id || hoveredId === link.to_event_id, bidirectional: Boolean(link.bidirectional), link });
  }
  const nodes = ordered.map(event => {
    const correlated = Boolean(selected) && event.correlation_id === selected.correlation_id;
    const causal = family.ancestors.has(event.id) || family.descendants.has(event.id);
    const hovered = event.id === hoveredId;
    const selectedNode = event.id === selectedId;
    const neighbor = neighbors.has(event.id);
    const dimmed = hoveredId ? !neighbor : Boolean(selected) && !selectedNode && !correlated && !causal;
    const color = FAMILY_COLORS[eventFamily(event)];
    const liveUntil = liveIds.get?.(event.id) ?? (liveIds.has?.(event.id) ? now + 6000 : 0);
    return {
      id: event.id,
      event,
      position: positions.get(event.id),
      color,
      radius: selectedNode || hovered ? 9 : 7,
      selected: selectedNode,
      correlated,
      causal,
      hovered,
      neighbor,
      dimmed,
      liveUntil,
      boundary: false,
      pickable: true,
    };
  });
  for (const { event, target } of missingParents) {
    const child = positions.get(event.id);
    const angle = (stableHash(event.causation_id) % 6283) / 1000;
    const source = nodes.length;
    const highlighted = selectedId === event.id || family.ancestors.has(event.id) || family.descendants.has(event.id);
    nodes.push({
      id: `boundary:${event.id}`,
      event: null,
      position: { x: child.x - Math.cos(angle) * 0.34, y: child.y - 0.32, z: child.z - Math.sin(angle) * 0.34 },
      color: [1, 0.8, 0.46, 1],
      radius: 8,
      selected: false,
      correlated: false,
      causal: highlighted,
      hovered: false,
      neighbor: false,
      dimmed: Boolean(selected) && !highlighted,
      liveUntil: 0,
      boundary: true,
      pickable: false,
      missingParentId: event.causation_id,
      childId: event.id,
    });
    edges.push({ source, target, kind: 0, highlighted, hovered: false, bidirectional: false, boundary: true });
  }
  return { nodes, edges, indexById, positions, family, boundaryCausal: missingParents.length, boundarySemantic };
}

export function packMemoryScene(scene, { wallNowMs = Date.now(), rendererNowSeconds = 0 } = {}) {
  const nodes = new Float32Array(Math.max(1, scene.nodes.length) * 12);
  scene.nodes.forEach((node, index) => {
    const offset = index * 12;
    nodes.set([node.position.x, node.position.y, node.position.z, node.radius, ...node.color], offset);
    let flags = 0;
    if (node.selected) flags |= 1;
    if (node.correlated) flags |= 2;
    if (node.causal) flags |= 4;
    if (node.hovered) flags |= 8;
    if (node.neighbor) flags |= 16;
    if (node.dimmed) flags |= 32;
    if (node.liveUntil > wallNowMs) flags |= 64;
    if (node.boundary) flags |= 128;
    const relativeExpiry = node.liveUntil ? rendererNowSeconds + (node.liveUntil - wallNowMs) / 1000 : 0;
    nodes.set([flags, relativeExpiry, 0, 0], offset + 8);
  });
  const edges = new Float32Array(Math.max(1, scene.edges.length) * 4);
  scene.edges.forEach((edge, index) => {
    let flags = edge.kind;
    if (edge.highlighted) flags += 2;
    if (edge.hovered) flags += 4;
    if (edge.bidirectional) flags += 8;
    edges.set([edge.source, edge.target, flags, 0], index * 4);
  });
  return { nodes, edges, nodeCount: scene.nodes.length, edgeCount: scene.edges.length };
}

export function needsInitialSceneFit(hasFittedNonEmptyScene, nodeCount) {
  return !hasFittedNonEmptyScene && nodeCount > 0;
}

export function pickMemoryNode(candidates, clientX, clientY, minimumRadius = 0) {
  let picked = null;
  for (const candidate of candidates) {
    if (!Number.isFinite(candidate.depth) || candidate.depth <= 0 || candidate.visible === false) continue;
    const distance = Math.hypot(clientX - candidate.x, clientY - candidate.y);
    if (distance > Math.max(minimumRadius, candidate.radius)) continue;
    if (!picked || candidate.depth < picked.depth || (candidate.depth === picked.depth && distance < picked.distance)) {
      picked = { id: candidate.id, depth: candidate.depth, distance };
    }
  }
  return picked?.id ?? null;
}

function stableHash(value) {
  let result = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 16777619);
  }
  return result >>> 0;
}
