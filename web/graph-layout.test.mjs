import assert from "node:assert/strict";
import test from "node:test";
import { createLiveBatch, createPermanentFailure, createRequestGate, expireLiveIds, intersectEventIds, pruneLiveIds } from "./graph-layout.mjs";

test("fatal graph state latches the first failure permanently", () => {
  let shutdowns = 0;
  const failure = createPermanentFailure(() => { shutdowns += 1; });
  const first = failure.latch(new Error("device lost"));
  const second = failure.latch(new Error("retry"));
  assert.equal(first.first, true);
  assert.equal(second.first, false);
  assert.equal(failure.error, first.error);
  assert.equal(failure.error.message, "device lost");
  assert.equal(shutdowns, 1, "fatal cleanup runs exactly once");
});

test("request generations reject stale graph responses", () => {
  const gate = createRequestGate();
  const first = gate.next();
  const second = gate.next();
  assert.equal(gate.isCurrent(first), false);
  assert.equal(gate.isCurrent(second), true);
  gate.invalidate();
  assert.equal(gate.isCurrent(second), false);
});

test("live arrivals batch, deduplicate, expire, and stay bounded to visible memories", () => {
  let scheduled = null;
  const flushes = [];
  const liveBatch = createLiveBatch(
    values => flushes.push(values),
    300,
    callback => { scheduled = callback; return 1; },
    () => { scheduled = null; },
  );
  liveBatch.add("event-1");
  liveBatch.add("event-1");
  liveBatch.add("event-2");
  scheduled();
  assert.deepEqual(flushes, [["event-1", "event-2"]]);
  liveBatch.add("event-3");
  liveBatch.clear();
  assert.equal(scheduled, null);

  const expirations = new Map([["expired", 10], ["current", 20]]);
  assert.deepEqual(expireLiveIds(expirations, 10), ["expired"]);
  assert.deepEqual(intersectEventIds(["missing", "current", "current"], [{ id: "current" }]), ["current"]);
  const rolling = new Map(Array.from({ length: 2000 }, (_, index) => [`event-${index}`, 100]));
  const window = Array.from({ length: 1000 }, (_, index) => ({ id: `event-${index + 1000}` }));
  assert.equal(pruneLiveIds(rolling, window).length, 1000);
  assert.equal(rolling.size, 1000);
});
