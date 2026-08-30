import assert from "node:assert/strict";
import { createRequestGate, fitTransform, layoutEvents, MIN_INTERACTIVE_SCALE, nearestNode, trimLine } from "./graph-layout.mjs";

const gate = createRequestGate();
const first = gate.next();
const second = gate.next();
assert.equal(gate.isCurrent(first), false, "an older graph request must become stale");
assert.equal(gate.isCurrent(second), true);
gate.invalidate();
assert.equal(gate.isCurrent(second), false, "leaving the graph invalidates the active request");

const events = Array.from({ length: 1000 }, (_, index) => ({
  id: `event-${index}`,
  sequence: index + 1,
  correlation_id: `correlation-${index % 40}`,
}));
const layout = layoutEvents(events);
assert.ok(layout.width > 30_000, "large layouts must not be compressed into a fixed world width");
for (let index = 1; index < events.length; index += 1) {
  assert.ok(layout.positions.get(events[index].id).x - layout.positions.get(events[index - 1].id).x >= 32);
}
const fitted = fitTransform(layout, { width: 1000, height: 700 });
assert.equal(fitted.scale, MIN_INTERACTIVE_SCALE, "fit must retain an interactive target size");
const newestScreenX = fitted.x + layout.positions.get(events.at(-1).id).x * fitted.scale;
assert.ok(newestScreenX > 900 && newestScreenX < 1000, "large fits should open near the newest events");

assert.deepEqual(
  trimLine({ x: 0, y: 0 }, { x: 100, y: 0 }, 8, 12),
  { source: { x: 8, y: 0 }, target: { x: 88, y: 0 } },
  "edge endpoints must stop outside source and target geometry",
);
assert.deepEqual(
  trimLine({ x: 0, y: 0 }, { x: 100, y: 0 }, 13, 13),
  { source: { x: 13, y: 0 }, target: { x: 87, y: 0 } },
  "bidirectional edges must leave room for arrowheads at both nodes",
);
assert.equal(trimLine({ x: 4, y: 4 }, { x: 4, y: 4 }, 8, 12), null);

const overlappingTargets = new Map([
  ["first", { x: 0, y: 0 }],
  ["second", { x: 4, y: 0 }],
]);
assert.equal(nearestNode(overlappingTargets, { x: 0.5, y: 0 }, 10), "first");
assert.equal(nearestNode(overlappingTargets, { x: 3.5, y: 0 }, 10), "second");
assert.equal(nearestNode(overlappingTargets, { x: 30, y: 0 }, 10), null);

console.log("graph layout: request gating, 1,000-node spacing, fit scale, and edge trimming passed");
