import { draw, effect, frameLoop, init, storage, surface, target } from "vgpu";
// The state module is plain ESM so Node can test the exact browser packing logic.
// @ts-expect-error no TypeScript declarations for the tested ESM helper
import { needsInitialSceneFit, packMemoryScene } from "./memory-graph-state.mjs";

type MemoryScene = {
  nodes: Array<{ selected: boolean; hovered: boolean; position: { x: number; y: number; z: number } }>;
  edges: Array<unknown>;
};

// The API returns at most 1,000 real memories; each may need one non-pickable parent boundary marker.
const MAX_NODES = 2000;
const MAX_EDGES = 3000;

export const NODE_SHADER = /* wgsl */ `
struct Node { position_radius: vec4f, color: vec4f, state: vec4f }
struct Camera { viewport: vec2f, rotation: vec2f, zoom: f32, time: f32, has_selection: f32, reduced_motion: f32 }
@group(0) @binding(0) var<storage, read> nodes: array<Node>;
@group(0) @binding(1) var<uniform> camera: Camera;
struct Out { @builtin(position) position: vec4f, @location(0) local: vec2f, @location(1) color: vec4f, @location(2) state: vec4f };
fn project(p: vec3f) -> vec3f {
  let cy = cos(camera.rotation.x); let sy = sin(camera.rotation.x);
  let ct = cos(camera.rotation.y); let st = sin(camera.rotation.y);
  let x = p.x * cy + p.z * sy;
  let z = -p.x * sy + p.z * cy;
  let y = p.y * ct - z * st;
  let z2 = p.y * st + z * ct;
  let depth = 4.2 + z2;
  let aspect = max(0.1, camera.viewport.x / camera.viewport.y);
  return vec3f(x / depth * camera.zoom / aspect, y / depth * camera.zoom, clamp(depth / 9.0, 0.01, 0.99));
}
@vertex fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  let corners = array<vec2f, 6>(vec2f(-1,-1), vec2f(1,-1), vec2f(-1,1), vec2f(-1,1), vec2f(1,-1), vec2f(1,1));
  let node = nodes[ii]; let center = project(node.position_radius.xyz); let corner = corners[vi];
  var radius = node.position_radius.w;
  let flags = u32(node.state.x);
  if ((flags & 64u) != 0u && camera.reduced_motion < 0.5) {
    let age = max(0.0, 6.0 - (node.state.y - camera.time));
    radius *= 1.0 + exp(-age * 1.8) * 1.8;
  }
  var out: Out;
  out.position = vec4f(center.xy + corner * radius * 2.0 / camera.viewport, center.z, 1.0);
  out.local = corner; out.color = node.color; out.state = node.state; return out;
}
@fragment fn fs_main(input: Out) -> @location(0) vec4f {
  let distance = length(input.local);
  let flags = u32(input.state.x);
  if ((flags & 128u) != 0u) {
    let diamond = abs(input.local.x) + abs(input.local.y);
    let dash = fract((atan2(input.local.y, input.local.x) + 3.14159265) * 2.55);
    if (diamond > 1.0 || diamond < 0.68 || dash > 0.7) { discard; }
    var alpha = select(0.26, 0.9, (flags & 4u) != 0u);
    if ((flags & 32u) != 0u) { alpha = 0.04; }
    return vec4f(1.0, 0.8, 0.46, alpha);
  }
  if (distance > 1.0) { discard; }
  var alpha = smoothstep(1.0, 0.72, distance);
  if ((flags & 32u) != 0u) { alpha *= 0.09; }
  var color = input.color.rgb;
  if ((flags & 1u) != 0u || (flags & 8u) != 0u) { color = vec3f(1.0); }
  if ((flags & 2u) != 0u) { color = mix(color, vec3f(0.72, 1.0, 0.8), 0.45); }
  if ((flags & 4u) != 0u) { color = mix(color, vec3f(1.0), 0.35); }
  let ring = select(0.0, smoothstep(0.72, 0.82, distance) * (1.0 - smoothstep(0.9, 1.0, distance)), (flags & 1u) != 0u || (flags & 64u) != 0u);
  return vec4f(color + ring * 0.45, max(alpha, ring));
}`;

export const EDGE_SHADER = /* wgsl */ `
struct Node { position_radius: vec4f, color: vec4f, state: vec4f }
struct Edge { data: vec4f }
struct Camera { viewport: vec2f, rotation: vec2f, zoom: f32, time: f32, has_selection: f32, reduced_motion: f32 }
@group(0) @binding(0) var<storage, read> nodes: array<Node>;
@group(0) @binding(1) var<storage, read> edges: array<Edge>;
@group(0) @binding(2) var<uniform> camera: Camera;
struct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f, @location(1) flags: f32 };
fn project(p: vec3f) -> vec3f {
  let cy = cos(camera.rotation.x); let sy = sin(camera.rotation.x);
  let ct = cos(camera.rotation.y); let st = sin(camera.rotation.y);
  let x = p.x * cy + p.z * sy; let z = -p.x * sy + p.z * cy;
  let y = p.y * ct - z * st; let z2 = p.y * st + z * ct; let depth = 4.2 + z2;
  let aspect = max(0.1, camera.viewport.x / camera.viewport.y);
  return vec3f(x / depth * camera.zoom / aspect, y / depth * camera.zoom, clamp(depth / 9.0 + 0.001, 0.01, 0.99));
}
@vertex fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {
  let edge = edges[ii].data; let a = project(nodes[u32(edge.x)].position_radius.xyz); let b = project(nodes[u32(edge.y)].position_radius.xyz);
  var uv = vec2f(0,-1); if (vi == 1u || vi == 4u || vi == 5u) { uv.x = 1; } if (vi == 2u || vi == 3u || vi == 5u) { uv.y = 1; }
  let point = mix(a.xy, b.xy, uv.x); let delta = (b.xy - a.xy) * camera.viewport; let normal = normalize(vec2f(-delta.y, delta.x));
  let bits = u32(edge.z); let width = select(4.0, 5.5, (bits & 2u) != 0u || (bits & 4u) != 0u);
  var out: Out; out.position = vec4f(point + normal * uv.y * width / camera.viewport, mix(a.z,b.z,uv.x), 1); out.uv = uv; out.flags = edge.z; return out;
}
@fragment fn fs_main(input: Out) -> @location(0) vec4f {
  let flags = u32(input.flags); let semantic = (flags & 1u) != 0u; let highlighted = (flags & 2u) != 0u; let hovered = (flags & 4u) != 0u; let bidirectional = (flags & 8u) != 0u;
  let shaft = abs(input.uv.y) < 0.22;
  let forward_arrow = input.uv.x > 0.78 && abs(input.uv.y) < (1.0 - input.uv.x) / 0.22;
  let reverse_arrow = bidirectional && input.uv.x < 0.22 && abs(input.uv.y) < input.uv.x / 0.22;
  if (!shaft && !forward_arrow && !reverse_arrow) { discard; }
  if (semantic && shaft && fract(input.uv.x * 12.0) > 0.58) { discard; }
  var color = select(vec3f(0.33,0.40,0.35), vec3f(0.47,0.71,1.0), semantic);
  if (highlighted || hovered) { color = vec3f(0.9,1.0,0.93); }
  var alpha = select(0.26, 0.82, highlighted || hovered); if (camera.has_selection > 0.5 && !highlighted && !hovered) { alpha = 0.035; }
  return vec4f(color, alpha);
}`;

export const COMPOSITE_SHADER = /* wgsl */ `
struct Params { texel: vec2f }
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@fragment fn fs_main(@location(0) uv: vec2f) -> @location(0) vec4f {
  return textureLoad(src, vec2u(vec2f(uv) / params.texel), 0);
}`;

export type RendererCallbacks = {
  onFatal(message: string): void;
};

export async function createMemoryGraphRenderer(canvas: HTMLCanvasElement, callbacks: RendererCallbacks) {
  const loopback = window.location.hostname === "localhost" || window.location.hostname === "127.0.0.1" || window.location.hostname === "[::1]";
  if (!loopback || !window.isSecureContext || !("gpu" in navigator)) {
    throw new Error("Live Memory Graph requires WebGPU on localhost. Open Habibi at http://localhost:8787 in a WebGPU-capable browser.");
  }
  const gpu = await init({ powerPreference: "high-performance" });
  const output = surface(gpu, canvas, { dpr: [1, 2] });
  let sceneTarget = target(gpu, { size: output.size, format: "rgba8unorm", depth: true, label: "memory-scene" });
  const nodeStorage = storage(gpu, MAX_NODES * 48, "read");
  const edgeStorage = storage(gpu, MAX_EDGES * 16, "read");
  const nodes = draw(gpu, { shader: NODE_SHADER, vertices: 6, blend: "alpha", depth: { write: true, compare: "less-equal" }, label: "memory-nodes" });
  const edges = draw(gpu, { shader: EDGE_SHADER, vertices: 6, blend: "alpha", depth: { write: false, compare: "less-equal" }, label: "memory-edges" });
  const composite = effect(gpu, COMPOSITE_SHADER, { label: "memory-composite" });
  let nodeCount = 0;
  let edgeCount = 0;
  let lastScene: MemoryScene | null = null;
  let hasFittedNonEmptyScene = false;
  let active = true;
  let disposed = false;
  let stopped = false;
  let fatalError: Error | null = null;
  let yaw = 0.55;
  let tilt = -0.24;
  let zoom = 1;
  let targetYaw: number | null = null;
  let targetTilt: number | null = null;
  let targetZoom: number | null = null;
  let hasSelection = false;
  let pauseSpin = false;
  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const monotonicOrigin = performance.now();
  let lastFrame = monotonicOrigin;
  const rendererNowSeconds = () => (performance.now() - monotonicOrigin) / 1000;

  const camera = () => ({ viewport: [Math.max(1, output.size[0]), Math.max(1, output.size[1])], rotation: [yaw, tilt], zoom, time: rendererNowSeconds(), has_selection: hasSelection ? 1 : 0, reduced_motion: reducedMotion ? 1 : 0 });
  const fitScene = (memoryScene: MemoryScene) => {
    const aspect = Math.max(0.1, output.size[0] / Math.max(1, output.size[1]));
    const cy = Math.cos(yaw), sy = Math.sin(yaw), ct = Math.cos(tilt), st = Math.sin(tilt);
    let extent = 0.1;
    for (const node of memoryScene.nodes) {
      const position = node.position;
      const x = position.x * cy + position.z * sy;
      const z = -position.x * sy + position.z * cy;
      const y = position.y * ct - z * st;
      const depth = 4.2 + position.y * st + z * ct;
      extent = Math.max(extent, Math.abs(x / depth / aspect), Math.abs(y / depth));
    }
    zoom = Math.min(4, 0.82 / extent);
  };
  const unresize = output.onResize(({ width, height }) => sceneTarget.resize([width, height]));
  const loop = frameLoop(gpu, frame => {
    if (!active || disposed || output.size[0] < 1 || output.size[1] < 1) return;
    const now = performance.now();
    const dt = Math.min(64, now - lastFrame);
    lastFrame = now;
    if (targetYaw !== null) {
      let delta = ((targetYaw - yaw + Math.PI) % (Math.PI * 2) + Math.PI * 2) % (Math.PI * 2) - Math.PI;
      yaw += delta * 0.12;
      tilt += ((targetTilt ?? tilt) - tilt) * 0.12;
      zoom += ((targetZoom ?? zoom) - zoom) * 0.12;
      if (Math.abs(delta) < 0.004) { targetYaw = null; targetTilt = null; targetZoom = null; }
    } else if (!reducedMotion && !pauseSpin) yaw += 0.00018 * dt;
    const values = camera();
    nodes.set({ nodes: nodeStorage, camera: values });
    edges.set({ nodes: nodeStorage, edges: edgeStorage, camera: values });
    frame.pass({ target: sceneTarget, clear: [0.018, 0.022, 0.04, 1], clearDepth: 1 }, pass => {
      if (edgeCount) pass.draw(edges, { instances: edgeCount });
      if (nodeCount) pass.draw(nodes, { instances: nodeCount });
    });
    composite.set({ src: sceneTarget.color, params: { texel: sceneTarget.texelSize } });
    frame.pass({ target: output }, pass => pass.draw(composite));
  });
  let stopErrors = () => {};
  const cleanup = () => {
    if (stopped) return;
    stopped = true;
    active = false;
    loop.stop();
    unresize();
    stopErrors();
    output.dispose();
    gpu.dispose();
  };
  const fail = (message: string) => {
    if (fatalError || disposed) return;
    fatalError = new Error(message);
    cleanup();
    callbacks.onFatal(message);
  };
  const assertOperational = () => {
    if (fatalError) throw fatalError;
    if (disposed) throw new Error("Memory graph renderer is disposed. Reload to retry.");
  };
  stopErrors = gpu.onError(error => fail(`WebGPU rendering failed: ${String(error)}`));
  void gpu.gpu.lost.then(info => fail(`WebGPU device lost: ${info.message || info.reason}`));

  return {
    get renderer() { return "vgpu/WebGPU"; },
    setScene(memoryScene: MemoryScene) {
      assertOperational();
      lastScene = memoryScene;
      if (needsInitialSceneFit(hasFittedNonEmptyScene, memoryScene.nodes.length)) {
        fitScene(memoryScene);
        hasFittedNonEmptyScene = true;
      }
      const packed = packMemoryScene(memoryScene, { wallNowMs: Date.now(), rendererNowSeconds: rendererNowSeconds() });
      if (packed.nodeCount > MAX_NODES || packed.edgeCount > MAX_EDGES) throw new Error("Memory graph exceeds its bounded GPU capacity.");
      nodeStorage.write(packed.nodes);
      edgeStorage.write(packed.edges);
      nodeCount = packed.nodeCount;
      edgeCount = packed.edgeCount;
      hasSelection = memoryScene.nodes.some(node => node.selected);
      pauseSpin = memoryScene.nodes.some(node => node.selected || node.hovered);
    },
    setActive(value: boolean) { if (value) assertOperational(); active = value; },
    rotate(deltaX: number, deltaY: number) {
      assertOperational();
      targetYaw = null; targetTilt = null; targetZoom = null;
      yaw += deltaX * 0.0055;
      tilt = Math.max(-1.1, Math.min(0.35, tilt - deltaY * 0.003));
    },
    zoom(factor: number) { assertOperational(); targetZoom = null; zoom = Math.max(0.5, Math.min(8, zoom * factor)); },
    fit() { assertOperational(); targetYaw = null; targetTilt = null; targetZoom = null; yaw = 0.55; tilt = -0.24; if (lastScene) fitScene(lastScene); },
    focus(position: { x: number; y: number; z: number }) {
      assertOperational();
      const nextYaw = Math.atan2(-position.x, position.z);
      const nextTilt = -0.18;
      const nextZoom = Math.max(zoom, 4.4);
      if (reducedMotion) {
        yaw = nextYaw; tilt = nextTilt; zoom = nextZoom;
        targetYaw = null; targetTilt = null; targetZoom = null;
      } else {
        targetYaw = nextYaw; targetTilt = nextTilt; targetZoom = nextZoom;
      }
    },
    project(position: { x: number; y: number; z: number }) {
      const cy = Math.cos(yaw), sy = Math.sin(yaw), ct = Math.cos(tilt), st = Math.sin(tilt);
      const x = position.x * cy + position.z * sy;
      const z = -position.x * sy + position.z * cy;
      const y = position.y * ct - z * st;
      const z2 = position.y * st + z * ct;
      const depth = 4.2 + z2;
      const width = Math.max(1, output.size[0]), height = Math.max(1, output.size[1]);
      return { x: width / 2 + x / depth * zoom / (width / height) * width / 2, y: height / 2 - y / depth * zoom * height / 2, depth };
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      cleanup();
    },
  };
}
