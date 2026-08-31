import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { draw, effect, frame, init, storage, target } from "vgpu/mock";
import { COMPOSITE_SHADER, EDGE_SHADER, NODE_SHADER } from "./generated/memory-graph.js";

test("distributed vgpu artifacts retain the adjacent MIT license notice", () => {
  const bundle = readFileSync(new URL("./generated/memory-graph.js", import.meta.url), "utf8");
  const license = readFileSync(new URL("./vgpu-LICENSE.txt", import.meta.url), "utf8");
  assert.match(bundle.slice(0, 100), /vgpu 0\.3\.1 \| MIT License \| \/assets\/vgpu-LICENSE\.txt/);
  assert.match(license, /Copyright \(c\) 2025 Vercel, Inc\./);
  assert.match(license, /Permission is hereby granted, free of charge/);
  assert.match(license, /THE SOFTWARE IS PROVIDED "AS IS"/);
});

test("vgpu reflects and records the memory graph instanced draws", async () => {
  const gpu = await init();
  const scene = target(gpu, { size: [64, 64], format: "rgba8unorm", depth: true });
  const output = target(gpu, { size: [64, 64], format: "rgba8unorm" });
  const nodeData = storage(gpu, 48, "read");
  const edgeData = storage(gpu, 16, "read");
  nodeData.write(new Float32Array([0, 0, 0, 7, 1, 1, 1, 1, 0, 0, 0, 0]));
  edgeData.write(new Float32Array([0, 0, 0, 0]));
  const nodes = draw(gpu, { shader: NODE_SHADER, vertices: 6, blend: "alpha", depth: { write: true, compare: "less-equal" } });
  const edges = draw(gpu, { shader: EDGE_SHADER, vertices: 6, blend: "alpha", depth: { write: false, compare: "less-equal" } });
  const copy = effect(gpu, COMPOSITE_SHADER);
  const camera = { viewport: [64, 64], rotation: [0, 0], zoom: 3, time: 0, has_selection: 0, reduced_motion: 1 };
  nodes.set({ nodes: nodeData, camera });
  edges.set({ nodes: nodeData, edges: edgeData, camera });
  frame(gpu, current => {
    current.pass({ target: scene, clear: [0, 0, 0, 1], clearDepth: 1 }, pass => {
      pass.draw(edges, { instances: 1 });
      pass.draw(nodes, { instances: 1 });
    });
    copy.set({ src: scene.color, params: { texel: scene.texelSize } });
    current.pass({ target: output }, pass => pass.draw(copy));
  });
  await gpu.settled();
  assert.equal(scene.size[0], 64);
  gpu.dispose();
});
