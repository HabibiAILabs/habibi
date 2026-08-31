import assert from "node:assert/strict";
import test from "node:test";
import { buildMemoryScene, causalFamily, memoryLayout, needsInitialSceneFit, packMemoryScene, pickMemoryNode } from "./memory-graph-state.mjs";

const events = [
  { id: "a", sequence: 1, event_type: "chat.message.created", source: "extension:chat", correlation_id: "c1", causation_id: null },
  { id: "b", sequence: 2, event_type: "action.requested", source: "habibi", correlation_id: "c1", causation_id: "a" },
  { id: "c", sequence: 3, event_type: "action.result.succeeded", source: "habibi", correlation_id: "c1", causation_id: "b" },
  { id: "d", sequence: 4, event_type: "test.other", source: "test", correlation_id: "c2", causation_id: null },
  { id: "e", sequence: 5, event_type: "test.child", source: "test", correlation_id: "c2", causation_id: "missing" },
];

test("memory layout is deterministic across input order and separates correlations in depth", () => {
  const left = memoryLayout(events);
  const right = memoryLayout([...events].reverse());
  assert.deepEqual([...left.positions], [...right.positions]);
  assert.notDeepEqual(left.positions.get("a"), left.positions.get("d"));
  assert.ok(left.positions.get("a").y < left.positions.get("c").y);
  assert.deepEqual(left.positions.get("c"), memoryLayout(events.slice(2)).positions.get("c"), "a memory must not move when older bounded-window events roll out");
});

test("chat memories in one session remain spatially grouped across turn correlations", () => {
  const sessionEvents = [
    { id: "s1", sequence: 10, event_type: "chat.message.created", source: "extension:chat", correlation_id: "turn-1", payload: { session_id: "shared" } },
    { id: "s2", sequence: 20, event_type: "chat.message.created", source: "tool:chat.send_message", correlation_id: "turn-2", payload: { session_id: "shared" } },
  ];
  const positions = memoryLayout(sessionEvents).positions;
  const left = positions.get("s1");
  const right = positions.get("s2");
  assert.ok(Math.hypot(left.x - right.x, left.z - right.z) < 0.5);
});

test("causal family is cycle-safe and reports a missing visible parent", () => {
  const family = causalFamily(events, "c");
  assert.deepEqual([...family.ancestors], ["b", "a"]);
  assert.deepEqual([...family.descendants], []);
  assert.equal(causalFamily(events, "e").omittedAncestors, 1);
  const cyclic = [
    { ...events[0], causation_id: "b" },
    { ...events[1], causation_id: "a" },
  ];
  assert.deepEqual([...causalFamily(cyclic, "a").ancestors], ["b"]);
});

test("scene keeps correlation separate from causal edges and packs bounded instances", () => {
  const scene = buildMemoryScene(events, [
    { link_id: "l", from_event_id: "c", to_event_id: "d", bidirectional: true },
    { link_id: "self", from_event_id: "c", to_event_id: "c", bidirectional: true },
  ], {
    selectedId: "c",
    hoveredId: "d",
    liveIds: new Map([["e", 9000]]),
    now: 5000,
  });
  assert.equal(scene.edges.filter(edge => edge.kind === 0).length, 3);
  assert.equal(scene.edges.filter(edge => edge.kind === 1).length, 1, "legacy self-links must not emit degenerate GPU geometry");
  assert.equal(scene.boundaryCausal, 1);
  assert.equal(scene.nodes.find(node => node.id === "b").causal, true);
  assert.equal(scene.nodes.find(node => node.id === "d").correlated, false);
  const boundary = scene.nodes.find(node => node.boundary);
  assert.equal(boundary.pickable, false);
  assert.equal(boundary.childId, "e");
  assert.equal(scene.edges.find(edge => edge.boundary).source, scene.nodes.indexOf(boundary));
  const packed = packMemoryScene(scene, { wallNowMs: 5000, rendererNowSeconds: 1 });
  assert.equal(packed.nodeCount, 6);
  assert.equal(packed.edgeCount, 4);
  assert.equal(packed.nodes.length, 72);
  assert.equal(packed.edges.length, 16);
  assert.equal(packed.nodes[4 * 12 + 9], 5, "GPU live deadlines are relative to the renderer clock origin");
  assert.equal(Number(packed.nodes[5 * 12 + 8]) & 128, 128, "GPU boundary instances are explicitly tagged");
});

test("boundary instances preserve the 1000-memory and 3000-edge GPU bounds", () => {
  const boundedEvents = Array.from({ length: 1000 }, (_, index) => ({
    id: `event-${index}`,
    sequence: index + 1,
    event_type: "test.event",
    source: "test",
    correlation_id: "bounded",
    causation_id: `missing-${index}`,
  }));
  const boundedLinks = Array.from({ length: 2000 }, (_, index) => ({
    link_id: `link-${index}`,
    from_event_id: "event-0",
    to_event_id: "event-1",
    bidirectional: false,
  }));
  const packed = packMemoryScene(buildMemoryScene(boundedEvents, boundedLinks));
  assert.equal(packed.nodeCount, 2000);
  assert.equal(packed.edgeCount, 3000);
});

test("live pulse packing preserves seconds at realistic epoch timestamps", () => {
  const now = Date.now();
  const rendererNowSeconds = 137.25;
  const scene = buildMemoryScene([events[0]], [], {
    liveIds: new Map([["a", now + 6000]]),
    now,
  });
  const packed = packMemoryScene(scene, { wallNowMs: now, rendererNowSeconds });
  const deadline = packed.nodes[9];
  assert.ok(deadline < 1000, `renderer deadline ${deadline} must never contain Unix epoch seconds`);
  assert.ok(Math.abs(deadline - 143.25) < 0.001, `relative deadline ${deadline} must retain millisecond-scale precision`);
  assert.ok(Math.abs((deadline - rendererNowSeconds) - 6) < 0.001, "the shader-visible pulse window remains six seconds at epoch-scale wall timestamps");
});

test("the first non-empty scene fits even after any number of empty scenes", () => {
  let fitted = false;
  assert.equal(needsInitialSceneFit(fitted, 0), false);
  assert.equal(needsInitialSceneFit(fitted, 0), false);
  assert.equal(needsInitialSceneFit(fitted, 3), true);
  fitted = true;
  assert.equal(needsInitialSceneFit(fitted, 4), false);
});

test("picking rejects behind-camera points and chooses the front-most rendered hit", () => {
  const candidates = [
    { id: "far", x: 100, y: 100, depth: 5, radius: 9 },
    { id: "near", x: 106, y: 100, depth: 3, radius: 7 },
    { id: "behind", x: 101, y: 100, depth: -0.5, radius: 20 },
    { id: "boundary", x: 101, y: 100, depth: 1, radius: 20, visible: false },
    { id: "miss", x: 200, y: 200, depth: 1, radius: 9 },
  ];
  assert.equal(pickMemoryNode(candidates, 101, 100, 8), "near");
  assert.equal(pickMemoryNode(candidates, 190, 190, 8), null);
  assert.equal(pickMemoryNode([{ id: "center", x: 100, y: 100, depth: 3, radius: 9 }, { id: "offset", x: 104, y: 100, depth: 3, radius: 9 }], 100, 100), "center");
});
