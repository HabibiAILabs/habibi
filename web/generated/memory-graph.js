/*! vgpu 0.3.1 | MIT License | /assets/vgpu-LICENSE.txt */
//#region node_modules/@vgpu/core/dist/errors.js
var e = class extends Error {
	code;
	severity;
	fix;
	where;
	cause;
	detail;
	constructor(e) {
		super(e.message, { cause: e.cause }), this.name = "VGPUError", this.code = e.code, this.severity = e.severity ?? "error", this.fix = e.fix, this.where = e.where, this.cause = e.cause, this.detail = e.detail;
	}
}, t = class extends e {
	constructor(e) {
		super({
			...e,
			severity: "error"
		}), this.name = "ValidationError";
	}
};
function n(t) {
	return new e({
		code: "VGPU-FEATURE-UNSUPPORTED",
		message: `Adapter does not support requested feature(s): ${t.map((e) => `"${e}"`).join(", ")}.`,
		fix: "Remove the unsupported name(s) from init({ requiredFeatures: [...] }) or run on an adapter that supports them; gate optional code paths on device.features after init.",
		where: "init"
	});
}
function r(e, t) {
	if (!e) return;
	let r = (t ?? []).filter((t) => !e.has(t));
	if (r.length) throw n(r);
}
//#endregion
//#region node_modules/@vgpu/core/dist/gpu-constants.js
var i = {
	map_read: 1,
	map_write: 2,
	copy_src: 4,
	copy_dst: 8,
	index: 16,
	vertex: 32,
	uniform: 64,
	storage: 128,
	indirect: 256,
	query_resolve: 512
};
function a(e) {
	let t = globalThis.GPUBufferUsage;
	return e.reduce((e, n) => e | o(n, t), 0);
}
function o(e, t) {
	let n = e.toUpperCase();
	return t?.[n] ?? i[e];
}
function s() {
	return globalThis.GPUMapMode?.READ ?? 1;
}
var c = {
	copy_src: 1,
	copy_dst: 2,
	texture_binding: 4,
	storage_binding: 8,
	render_attachment: 16
};
function l(e) {
	let t = globalThis.GPUTextureUsage;
	return e.reduce((e, n) => e | u(n, t), 0);
}
function u(e, t) {
	let n = e.toUpperCase();
	return t?.[n] ?? c[e];
}
//#endregion
//#region node_modules/@vgpu/core/dist/mock-gpu-storage.js
function d(e) {
	return "__vgpuMockBytes" in e;
}
function f(e) {
	return "__vgpuMockBytes" in e;
}
//#endregion
//#region node_modules/@vgpu/core/dist/resource-lifecycle.js
var p = 1;
function m(e) {
	return Object.freeze({
		kind: e,
		id: p++
	});
}
var h = class {
	callbacks = /* @__PURE__ */ new Set();
	destroyed = !1;
	onDestroy(e, t) {
		return this.destroyed ? (t(e), () => void 0) : (this.callbacks.add(t), () => {
			this.callbacks.delete(t);
		});
	}
	emit(e) {
		if (this.destroyed) return !1;
		this.destroyed = !0;
		let t = [...this.callbacks];
		this.callbacks.clear();
		for (let n of t) n(e);
		return !0;
	}
}, g = class {
	device;
	gpu;
	options;
	ownership;
	destroySignal = new h();
	identity = m("buffer");
	destroyed = !1;
	constructor(e, t, n, r = "owned") {
		this.device = e, this.gpu = t, this.options = n, this.ownership = r, Object.defineProperty(this, "assertUsable", { value: (e) => this.#e(e) });
	}
	get resourceIdentity() {
		return this.identity;
	}
	onDestroy(e) {
		return this.destroySignal.onDestroy(this, e);
	}
	#e(e = "Buffer") {
		if (this.destroyed) throw new t({
			code: "VGPU-BUFFER-DISPOSED",
			message: "Buffer is destroyed.",
			where: e,
			fix: "Wrap or create a live GPUBuffer before using it."
		});
		this.device.assertUsable(e);
	}
	write(e, t = 0) {
		this.#e("Buffer.write"), this.ownership === "external" && this.validateExternalOperation("write", t, e.byteLength, "copy_dst");
		try {
			this.device.queue.writeBuffer(this.gpu, t, e);
		} catch (e) {
			throw this.ownership === "external" ? _("Buffer.write", "The external GPUBuffer rejected the write operation.", e) : e;
		}
	}
	async read(e, n = 0) {
		this.#e("Buffer.read"), this.ownership === "external" && this.validateExternalOperation("read", n, e, "copy_src");
		try {
			let t = await this.device.readback.read(this.gpu, e, n);
			return this.#e("Buffer.read"), t;
		} catch (e) {
			throw e instanceof t || this.ownership !== "external" ? e : _("Buffer.read", "The external GPUBuffer rejected the read operation.", e);
		}
	}
	destroy() {
		this.destroyed || (this.destroyed = !0, this.destroySignal.emit(this), this.ownership === "owned" && !d(this.gpu) && this.gpu.destroy());
	}
	dispose() {
		this.destroy();
	}
	validateExternalOperation(e, t, n, r) {
		if (!(Number.isSafeInteger(t) && t >= 0 && t % 4 == 0 && Number.isSafeInteger(n) && n >= 0 && n % 4 == 0 && t <= this.options.size && n <= this.options.size - t)) throw _(`Buffer.${e}`, "External buffer offsets and lengths must be non-negative, 4-byte aligned, and within the buffer size.");
		if ((this.gpu.usage & a([r])) === 0) throw _(`Buffer.${e}`, `External buffer is missing ${r.toUpperCase()} usage.`);
	}
};
function _(e, n, r) {
	return new t({
		code: "VGPU-EXTERNAL-BUFFER-VALIDATION",
		message: n,
		where: e,
		cause: r,
		fix: "Use a buffer with the required usage flags and an aligned in-range operation."
	});
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/compile.js
function v(e) {
	if (x(e)) throw S();
	let t = {
		version: 1,
		mappings: []
	}, n = {
		version: 1,
		modules: [{
			path: "<runtime>",
			text: e
		}],
		diagnostics: [],
		sourceMap: t,
		cacheKey: y(e)
	};
	return {
		kind: "wgsl",
		wgsl: e,
		source: {
			text: e,
			path: "<runtime>",
			imports: []
		},
		ast: n,
		sourceMap: t,
		diagnostics: [],
		cacheKey: n.cacheKey,
		entryPoints: b(e),
		stats: {
			lines: e.split(/\r?\n/).length,
			bytes: new TextEncoder().encode(e).byteLength,
			bindGroups: 0
		}
	};
}
function y(e) {
	let t = 2166136261;
	for (let n = 0; n < e.length; n++) t = Math.imul(t ^ e.charCodeAt(n), 16777619);
	return { default: `vgpu-wgsl-1:${(t >>> 0).toString(16).padStart(8, "0")}` };
}
function b(e) {
	let t = [];
	for (let n of e.matchAll(/@(vertex|fragment|compute)\s+fn\s+([A-Za-z_][A-Za-z0-9_]*)/g)) t.push(n[2]);
	return t;
}
function x(e) {
	let t = e.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "").trimStart();
	return t.startsWith("import ") || t.startsWith("import{");
}
function S() {
	let e = /* @__PURE__ */ Error("Runtime WGSL strings cannot contain import statements. Use a build-time loader or @vgpu/wgsl/runtime.");
	return e.name = "VGPUWGSLRuntimeImportError", e.code = "VGPU-WGSL-RUNTIME-IMPORT", e.severity = "error", e.source = "wgsl", e;
}
//#endregion
//#region node_modules/@vgpu/core/dist/readback.js
var C = a(["copy_dst", "map_read"]), ee = class {
	device;
	constructor(e) {
		this.device = e;
	}
	async read(e, t, n) {
		if (d(e)) return e.__vgpuMockBytes.slice(n, n + t).buffer;
		let r = this.device.createBuffer({
			size: t,
			usage: C
		});
		try {
			let i = this.device.createCommandEncoder();
			i.copyBufferToBuffer(e, n, r, 0, t), this.device.queue.submit([i.finish()]), await r.mapAsync(s());
			let a = r.getMappedRange().slice(0);
			return te(r), a;
		} finally {
			ne(r);
		}
	}
	async readTexture(e, t, n) {
		let [r, i] = t, a = w(n, "Readback.readTexture"), o = a.bytesPerPixel, c = re(r * o, 256), l = c * i, u = this.device.createBuffer({
			size: l,
			usage: C
		}), d;
		try {
			let t = this.device.createCommandEncoder();
			t.copyTextureToBuffer({ texture: e }, {
				buffer: u,
				bytesPerRow: c,
				rowsPerImage: i
			}, {
				width: r,
				height: i
			}), this.device.queue.submit([t.finish()]), await u.mapAsync(s());
			let n = new Uint8Array(u.getMappedRange());
			d = new Uint8Array(r * i * o);
			for (let e = 0; e < i; e++) {
				let t = e * c, i = e * r * o;
				d.set(n.subarray(t, t + r * o), i);
			}
			te(u);
		} finally {
			ne(u);
		}
		return a.swizzle === "bgra-to-rgba" && ce(d), d;
	}
	destroy() {}
};
function te(e) {
	try {
		e.unmap();
	} catch {}
}
function ne(e) {
	try {
		e.destroy();
	} catch {}
}
function re(e, t) {
	return Math.ceil(e / t) * t;
}
var ie = {
	r8unorm: {
		bytesPerPixel: 1,
		components: 1,
		componentType: "unorm8"
	},
	rg8unorm: {
		bytesPerPixel: 2,
		components: 2,
		componentType: "unorm8"
	},
	rgba8unorm: {
		bytesPerPixel: 4,
		components: 4,
		componentType: "unorm8"
	},
	"rgba8unorm-srgb": {
		bytesPerPixel: 4,
		components: 4,
		componentType: "unorm8"
	},
	bgra8unorm: {
		bytesPerPixel: 4,
		components: 4,
		componentType: "unorm8",
		swizzle: "bgra-to-rgba"
	},
	"bgra8unorm-srgb": {
		bytesPerPixel: 4,
		components: 4,
		componentType: "unorm8",
		swizzle: "bgra-to-rgba"
	},
	r16float: {
		bytesPerPixel: 2,
		components: 1,
		componentType: "float16"
	},
	rg16float: {
		bytesPerPixel: 4,
		components: 2,
		componentType: "float16"
	},
	rgba16float: {
		bytesPerPixel: 8,
		components: 4,
		componentType: "float16"
	},
	r32float: {
		bytesPerPixel: 4,
		components: 1,
		componentType: "float32"
	},
	rg32float: {
		bytesPerPixel: 8,
		components: 2,
		componentType: "float32"
	},
	rgba32float: {
		bytesPerPixel: 16,
		components: 4,
		componentType: "float32"
	}
};
function w(e, n) {
	let r = ie[e];
	if (r) return r;
	throw new t({
		code: "VGPU-CORE-UNSUPPORTED-FORMAT",
		message: `Texture.read does not support format ${e}. Supported formats: ${Object.keys(ie).join(", ")}.`,
		where: n
	});
}
function ae(e, t, n = "Texture.readFloats") {
	let r = w(t, n), i = r.bytesPerPixel / r.components, a = Math.floor(e.byteLength / i), o = new Float32Array(a), s = new DataView(e.buffer, e.byteOffset, e.byteLength);
	for (let e = 0; e < a; e++) r.componentType === "unorm8" ? o[e] = s.getUint8(e) / 255 : r.componentType === "float16" ? o[e] = oe(s.getUint16(e * 2, !0)) : o[e] = s.getFloat32(e * 4, !0);
	return o;
}
function oe(e) {
	let t = e & 32768 ? -1 : 1, n = e >> 10 & 31, r = e & 1023;
	return n === 0 ? t * r * 2 ** -24 : n === 31 ? r === 0 ? t * Infinity : NaN : t * (r + 1024) * 2 ** (n - 25);
}
function se(e, t, n) {
	let r = e.slice(0, t[0] * t[1] * n.bytesPerPixel);
	return n.swizzle === "bgra-to-rgba" && ce(r), r;
}
function ce(e) {
	for (let t = 0; t < e.length; t += 4) {
		let n = e[t];
		e[t] = e[t + 2], e[t + 2] = n;
	}
}
//#endregion
//#region node_modules/@vgpu/core/dist/mock-gpu.js
function le(e) {
	return {
		size: e,
		usage: a(["copy_src", "copy_dst"])
	};
}
//#endregion
//#region node_modules/@vgpu/core/dist/queue.js
var ue = class {
	gpu;
	guard;
	constructor(e, t = () => void 0) {
		this.gpu = e, this.guard = t;
	}
	writeBuffer(e, t, n) {
		this.guard("Queue.writeBuffer"), this.gpu.writeBuffer(e, t, n);
	}
	async flush() {
		this.guard("Queue.flush"), await this.gpu.onSubmittedWorkDone?.(), this.guard("Queue.flush");
	}
}, de = class {
	gpu;
	resolved;
	constructor(e, t) {
		this.gpu = e, this.resolved = t;
	}
	dispose() {}
	get kind() {
		return this.resolved.kind;
	}
	get source() {
		return this.resolved.source;
	}
	get code() {
		return this.resolved.wgsl;
	}
	get entryPoints() {
		return this.resolved.entryPoints;
	}
	get stats() {
		return this.resolved.stats;
	}
}, T = Symbol.for("vgpu/Texture"), fe = Symbol.for("vgpu/Texture/resizeLock"), pe = class {
	device;
	ownership;
	[T] = !0;
	destroySignal = new h();
	identity = m("texture");
	currentGpu;
	currentOptions;
	defaultView = null;
	resizeLock;
	destroyed = !1;
	constructor(e, t, n, r = "owned") {
		this.device = e, this.ownership = r, this.currentGpu = t, this.currentOptions = n, Object.defineProperty(this, fe, { value: (e) => {
			this.resizeLock = e;
		} });
	}
	get gpu() {
		return this.currentGpu;
	}
	get options() {
		return this.currentOptions;
	}
	get size() {
		return this.options.size;
	}
	get format() {
		return this.options.format;
	}
	get usage() {
		return this.options.usage;
	}
	get mipLevelCount() {
		return this.options.mipLevelCount ?? 1;
	}
	get sampleCount() {
		return this.options.sampleCount ?? 1;
	}
	get dimension() {
		return this.options.dimension ?? "2d";
	}
	get viewFormats() {
		return this.options.viewFormats ?? [];
	}
	get label() {
		return this.options.label;
	}
	get resourceIdentity() {
		return this.identity;
	}
	onDestroy(e) {
		return this.destroySignal.onDestroy(this, e);
	}
	get view() {
		return this.assertAlive(), this.defaultView ??= this.createView(), this.defaultView;
	}
	createView(e) {
		return this.assertAlive("Texture.createView"), this.gpu.createView(e);
	}
	resize(e) {
		if (this.assertAlive(), this.ownership === "external") throw new t({
			code: "VGPU-CORE-EXTERNAL-TEXTURE",
			message: "Texture wraps an externally owned GPUTexture and cannot be resized.",
			where: "Texture.resize"
		});
		if (this.resizeLock) throw new t({
			code: "VGPU-CORE-TEXTURE-RESIZE-LOCKED",
			message: this.resizeLock,
			where: "Texture.resize"
		});
		let n = this.options.size[2] ?? 1, r = e[2] ?? n;
		if (this.options.size[0] === e[0] && this.options.size[1] === e[1] && n === r) return !1;
		let i = e[2] === void 0 && this.options.size[2] === void 0 ? [e[0], e[1]] : [
			e[0],
			e[1],
			r
		], a = {
			...this.options,
			size: i
		}, o = this.gpu;
		return this.currentGpu = this.device.gpu.createTexture(me(a)), this.currentOptions = a, this.defaultView = null, o.destroy(), !0;
	}
	async read() {
		this.assertAlive("Texture.read");
		let e = w(this.options.format, "Texture.read");
		if (f(this.gpu)) return se(this.gpu.__vgpuMockBytes, this.options.size, e);
		let t = await this.device.readback.readTexture(this.gpu, this.options.size, this.options.format);
		return this.assertAlive("Texture.read"), t;
	}
	async readFloats() {
		return w(this.options.format, "Texture.readFloats"), ae(await this.read(), this.options.format);
	}
	destroy() {
		this.destroyed || (this.destroyed = !0, this.defaultView = null, this.destroySignal.emit(this), this.ownership !== "external" && (f(this.gpu) || this.gpu.destroy()));
	}
	dispose() {
		this.destroy();
	}
	assertAlive(e = "Texture") {
		if (this.destroyed) throw new t({
			code: "VGPU-CORE-TEXTURE-DESTROYED",
			message: "Texture is destroyed",
			where: e
		});
		this.device.assertUsable?.(e);
	}
};
function me(e) {
	let t = {
		label: e.label,
		size: {
			width: e.size[0],
			height: e.size[1],
			depthOrArrayLayers: e.size[2] ?? 1
		},
		format: e.format,
		usage: l(e.usage)
	};
	return e.mipLevelCount !== void 0 && (t.mipLevelCount = e.mipLevelCount), e.sampleCount !== void 0 && (t.sampleCount = e.sampleCount), e.dimension !== void 0 && (t.dimension = e.dimension), e.viewFormats !== void 0 && (t.viewFormats = [...e.viewFormats]), t;
}
//#endregion
//#region node_modules/@vgpu/core/dist/device.js
var he = class {
	gpu;
	adapterInfo;
	queue;
	readback;
	isCompatibilityMode;
	scopes = [];
	ownership;
	state = "alive";
	lossInfo;
	observeLoss = !0;
	constructor(e, t = null, n = "owned", r = {}) {
		this.gpu = e, this.adapterInfo = t, Object.defineProperty(this, "assertUsable", { value: (e) => this.#e(e) }), this.ownership = typeof n == "string" ? n : "owned";
		let i = typeof n == "string" ? r : n;
		this.isCompatibilityMode = i.isCompatibilityMode ?? !1, this.queue = new ue(e.queue, (e) => this.#e(e)), this.readback = new ee(e);
		let a = e.lost;
		a && typeof a.then == "function" && Promise.resolve(a).then((e) => {
			!this.observeLoss || this.state !== "alive" || (this.lossInfo = e, this.state = "lost");
		}, () => void 0);
	}
	get limits() {
		return this.#e("Device.limits"), this.gpu.limits;
	}
	get features() {
		return this.#e("Device.features"), this.gpu.features;
	}
	createShader(e) {
		this.#e("Device.createShader");
		let t = typeof e == "string" ? v(e) : e;
		return new de(this.gpu.createShaderModule({ code: t.wgsl }), t);
	}
	createTexture(e) {
		return this.#e("Device.createTexture"), new pe(this, this.gpu.createTexture(me(e)), e);
	}
	createBuffer(e) {
		this.#e("Device.createBuffer");
		let t = ge(e);
		t && this.captureError(t);
		let n = t ? le(Math.max(4, e.size || 4)) : ve(e);
		return new g(this, this.gpu.createBuffer(n), e);
	}
	wrapBuffer(e) {
		if (this.#e("Device.wrapBuffer"), !be(e)) throw new t({
			code: "VGPU-EXTERNAL-BUFFER-INVALID",
			message: "Device.wrapBuffer requires a GPUBuffer with finite size and usage properties.",
			where: "Device.wrapBuffer",
			fix: "Pass a live GPUBuffer created for this GPUDevice."
		});
		let n = {
			size: e.size,
			usage: Se(e.usage),
			...e.label ? { label: e.label } : {}
		};
		return new g(this, e, n, "external");
	}
	pushErrorScope(e) {
		this.#e("Device.pushErrorScope"), this.scopes.push([]), this.gpu.pushErrorScope?.(e);
	}
	async popErrorScope() {
		this.#e("Device.popErrorScope");
		let e = this.scopes.pop(), t = await this.gpu.popErrorScope?.();
		return this.#e("Device.popErrorScope"), e?.[0] ?? ye(t) ?? null;
	}
	#e(e) {
		if (this.state === "alive") return;
		if (this.state === "disposed") throw new t({
			code: "VGPU-DEVICE-DISPOSED",
			message: "The GPU device wrapper has been disposed.",
			where: e,
			fix: "Create a new Gpu instance before performing more work."
		});
		let n = this.lossInfo?.reason, r = this.lossInfo?.message;
		throw new t({
			code: "VGPU-DEVICE-LOST",
			message: `The GPU device was lost${n ? ` (${n})` : ""}${r ? `: ${r}` : "."}`,
			where: e,
			cause: this.lossInfo
		});
	}
	destroy() {
		if (this.state === "disposed") return;
		let e = this.state === "lost";
		this.state = "disposed", this.observeLoss = !1, this.scopes.length = 0, this.readback.destroy(), this.ownership === "owned" && !e && this.gpu.destroy();
	}
	dispose() {
		this.destroy();
	}
	captureError(e) {
		let t = this.scopes.at(-1);
		if (t) t.push(e);
		else throw e;
	}
};
function ge(e) {
	return !Number.isFinite(e.size) || e.size <= 0 ? _e("Buffer size must be greater than zero.") : e.usage.length === 0 ? _e("Buffer usage must not be empty.") : null;
}
function _e(e) {
	return new t({
		code: "VGPU-CORE-INVALID-USAGE",
		message: e,
		where: "Device.createBuffer"
	});
}
function ve(e) {
	return {
		label: e.label,
		size: e.size,
		usage: a(e.usage)
	};
}
function ye(e) {
	return e ? new t({
		code: "VGPU-CORE-VALIDATION",
		message: e.message,
		where: "GPUDevice.popErrorScope",
		cause: e
	}) : null;
}
function be(e) {
	if (typeof e != "object" && typeof e != "function" || e === null) return !1;
	let t = e;
	return Number.isSafeInteger(t.size) && (t.size ?? -1) >= 0 && Number.isSafeInteger(t.usage) && (t.usage ?? -1) >= 0 && typeof t.destroy == "function";
}
var xe = [
	"map_read",
	"map_write",
	"copy_src",
	"copy_dst",
	"index",
	"vertex",
	"uniform",
	"storage",
	"indirect",
	"query_resolve"
];
function Se(e) {
	return xe.filter((t) => (e & a([t])) !== 0);
}
//#endregion
//#region node_modules/@vgpu/core/dist/bind-group-metadata.js
var Ce = /* @__PURE__ */ new WeakMap(), we = /* @__PURE__ */ new WeakMap();
function Te(e, t) {
	return Ce.set(e, Oe(t)), e;
}
function Ee(e) {
	return Ce.get(e);
}
function De(e) {
	return we.get(e);
}
function Oe(e) {
	return { entries: e.entries.map((e) => ({ ...e })) };
}
//#endregion
//#region node_modules/vgpu/dist/errors.js
var E = class extends e {};
function ke(e, t, n, r, i, a) {
	let o = t === "vertex" ? "Vertex" : "Fragment", s = t === "vertex" ? "VERTEX" : "FRAGMENT", c = `maxStorageBuffersIn${o}Stage`;
	return new E({
		code: `VGPU-LIMIT-STORAGE-${s}`,
		message: `${o} entry '${n}' in '${e}' uses ${r} storage buffer(s), but device limit ${c} is ${i}.`,
		fix: t === "vertex" ? `Request init({ requiredLimits: { ${c}: ${r} } }) if the adapter supports it, or move vertex data to geometry(gpu, ...) vertex streams.` : `Request init({ requiredLimits: { ${c}: ${r} } }) if the adapter supports it, or reduce fragment storage buffers.`,
		where: `${e}.pipelineLayout`,
		detail: {
			stage: t,
			entryPoint: n,
			count: r,
			limit: i,
			bindings: a.map(({ name: e, group: t, binding: n }) => ({
				name: e,
				group: t,
				binding: n
			}))
		}
	});
}
function Ae(e, t, n, r, i) {
	return new E({
		code: "VGPU-SET-TEXTURE-FILTERABILITY",
		message: `${r} (${n}) cannot satisfy filtering texture '${t.name}' @group(${t.group}) @binding(${t.binding}).`,
		fix: "Use a filterable format; request float32-filterable for rgba32float when supported; or use textureLoad without a sampler.",
		where: `${e}.set`,
		detail: {
			format: n,
			group: t.group,
			binding: t.binding,
			bindingName: t.name,
			resourceName: r,
			samplerName: i?.name,
			samplerGroup: i?.group,
			samplerBinding: i?.binding
		}
	});
}
function je(e, t) {
	let n = Dt(e, t);
	return new E({
		code: "VGPU-R1-BINDING-NEVER-SET",
		message: `Unset \`${t.name}\` @group(${t.group}) @binding(${t.binding}) in '${e}'. Fix: ${n}; or ${e}.group(${t.group}, bindGroup).`,
		where: `${e}.draw`
	});
}
function Me(e, t) {
	return new E({
		code: "VGPU-R1-OWNERSHIP-FLIP",
		message: `\`${e}\` is ${t === "lib" ? "lib-owned by its first JS set()" : "user-owned by its first resource set()"}; ownership cannot change. ${t === "lib" ? `Fix: pass a resource from the start: wave.set({ ${e}: new Uniform(gpu.device, { size: 4 }) }).` : `Fix: pass JS values from the first set(): wave.set({ ${e}: jsValue }).`}`,
		where: "set"
	});
}
function Ne(e, t) {
	return new E({
		code: "VGPU-R4-GROUP-CLAIMED",
		message: `group ${t} of '${e}' is claimed; set() cannot update it.`,
		fix: `Call set() first, or build from ${e}.layout(${t}); pass dynamic offsets to p.draw().`,
		where: `${e}.set`
	});
}
function Pe(e, t, n, r) {
	return new E({
		code: "VGPU-R4-GROUP-INCOMPATIBLE",
		message: `claimed group ${t} in '${e}' is incompatible: ${n}.`,
		fix: `Build from ${e}.layout(${t}, { dynamicOffsets? }) then call ${e}.group(${t}, bindGroup).`,
		where: `${e}.group`,
		cause: r
	});
}
function D(e, t, n) {
	return new E({
		code: "VGPU-R4-GROUP-VALIDATION",
		message: `WebGPU rejected claimed group ${t} in '${e}'.`,
		fix: `Build from ${e}.layout(${t}); pass offsets via p.draw(draw, { offsets: { ${t}: [...] } }).`,
		where: `${e}.draw`,
		cause: n,
		detail: {
			drawLabel: e,
			group: t
		}
	});
}
function Fe(e, t) {
	return new E({
		code: "VGPU-BLEND-INVALID",
		message: `Invalid blend '${String(t)}' in '${e}'.`,
		fix: "Use \"alpha\", \"additive\", \"premultiplied\", or { color, alpha? } components.",
		where: "draw"
	});
}
function Ie(e, t) {
	return new E({
		code: "VGPU-BLEND-CONSTANT-INVALID",
		message: `Invalid blendConstant in '${e}': ${t}`,
		fix: "Use [r, g, b, a] finite numbers with a blend whose color or alpha uses \"constant\"/\"one-minus-constant\"; omit it to keep the pass default (0, 0, 0, 0).",
		where: "draw"
	});
}
function Le(e, t) {
	return new E({
		code: "VGPU-WRITEMASK-INVALID",
		message: `Invalid writeMask ${t} in '${e}'.`,
		fix: "Use an array of r/g/b/a; omit it for all channels.",
		where: "draw"
	});
}
function Re(e, t, n = "draw") {
	return new E({
		code: "VGPU-COLORS-INVALID",
		message: `Invalid colors in '${e}': ${t}`,
		fix: "Use one { blend?, writeMask? } or null entry per color attachment of the target, aligned by index; omit colors to apply the top-level blend/writeMask to every attachment.",
		where: n
	});
}
function ze(e, t) {
	return new E({
		code: "VGPU-CULL-INVALID",
		message: `Invalid cull '${String(t)}' in '${e}'.`,
		fix: "Use \"none\", \"front\", or \"back\"; omit it for no culling.",
		where: "draw"
	});
}
function Be(e, t) {
	return new E({
		code: "VGPU-FRONTFACE-INVALID",
		message: `Invalid frontFace '${String(t)}' in '${e}'.`,
		fix: "Use \"ccw\" or \"cw\"; omit it for counter-clockwise.",
		where: "draw"
	});
}
function Ve(e, t) {
	return new E({
		code: "VGPU-UNCLIPPED-DEPTH-INVALID",
		message: `Invalid unclippedDepth in '${e}': ${t}`,
		fix: "Use a boolean. unclippedDepth: true needs the \"depth-clip-control\" device feature — request it with init({ requiredFeatures: [\"depth-clip-control\"] }) on an adapter that supports it. Omit the option to keep depth clipping.",
		where: "draw"
	});
}
function O(e, t) {
	return new E({
		code: "VGPU-DEPTH-INVALID",
		message: `Invalid depth in '${e}': ${t}`,
		fix: "Use false or { write?, compare?, bias?, biasSlopeScale?, biasClamp? }; omit it for { write: true, compare: \"less-equal\" }.",
		where: "draw"
	});
}
function He(e, t, n = "draw") {
	return new E({
		code: "VGPU-STENCIL-INVALID",
		message: `Invalid stencil in '${e}': ${t}`,
		fix: "Use { front?, back?, readMask?, writeMask?, ref? } with GPUCompareFunction/GPUStencilOperation faces and u32 masks, against a target whose depth format has a stencil aspect (depth: \"depth24plus-stencil8\"); omit it for WebGPU's pass-through defaults.",
		where: n
	});
}
function Ue(e, t, n = "draw") {
	return new E({
		code: "VGPU-MULTISAMPLE-INVALID",
		message: `Invalid multisample in '${e}': ${t}`,
		fix: "Use { alphaToCoverage?, mask? }: alphaToCoverage needs a target created with msaa: true, and mask must be an integer in [0, 0xFFFFFFFF] (bits above the target's sampleCount are ignored). Omit multisample for full-coverage defaults.",
		where: n
	});
}
function We(e, t, n = "draw") {
	return new E({
		code: "VGPU-CONSTANTS-INVALID",
		message: `Invalid constants in '${e}': ${t}`,
		fix: "Key WGSL `override` constants by name, or by the decimal string of N when the declaration has @id(N); values are finite numbers or booleans, converted to the override's WGSL type (bool/i32/u32/f32/f16). Every override without a default value must be provided. Omit constants to keep the WGSL defaults.",
		where: n
	});
}
function Ge(e, t, n = "draw") {
	return new E({
		code: "VGPU-ENTRY-INVALID",
		message: `Invalid entry in '${e}': ${t}`,
		fix: "Name an entry point declared in the shader with the matching stage — { vertex?, fragment? } strings for draw, one @compute name string for compute. Omit entry (or a field) to use the first entry point of that stage.",
		where: n
	});
}
function Ke(e, t, n) {
	return new E({
		code: "VGPU-INDIRECT-INVALID",
		message: `Invalid indirect in '${e}': ${t}`,
		fix: "Pass a storage buffer created with storage(gpu, bytes, { indirect: true }) — bare, or as { buffer, offset? } with a 4-aligned byte offset — sized so the GPU-read arguments fit: 16 bytes for drawIndirect, 20 for drawIndexedIndirect, 12 for dispatchWorkgroupsIndirect. Omit indirect to use CPU-side counts.",
		where: n
	});
}
function qe() {
	return new E({
		code: "VGPU-PASS-PRESERVE-MSAA",
		message: "clear:false cannot preserve MSAA; use a non-MSAA target.",
		fix: "Use non-MSAA for accumulation.",
		where: "Frame.pass"
	});
}
function Je(e, t = "expected a number in [0, 1].", n = "Use 1 (default), or 0 with depth: { compare: \"greater\" } for reversed-Z.") {
	return new E({
		code: "VGPU-PASS-CLEARDEPTH-INVALID",
		message: `clearDepth received ${String(e)}; ${t}`,
		fix: n,
		where: "Frame.pass"
	});
}
function k(e) {
	return new E({
		code: "VGPU-PASS-VIEWPORT-INVALID",
		message: `Invalid viewport: ${e}`,
		fix: "Use { x?, y?, width, height, minDepth?, maxDepth? } finite numbers within device limits; omit it for the full target.",
		where: "Frame.pass"
	});
}
function Ye(e) {
	return new E({
		code: "VGPU-PASS-SCISSOR-INVALID",
		message: `Invalid scissor: ${e}`,
		fix: "Use [x, y, width, height] non-negative integers with x + width and y + height within the target's current pixel size; omit it for the full target.",
		where: "Frame.pass"
	});
}
function Xe() {
	return new E({
		code: "VGPU-PASS-PRESERVE-CLEARDEPTH",
		message: "clear:false preserves depth; clearDepth cannot apply.",
		fix: "Remove clearDepth, or let the pass clear.",
		where: "Frame.pass"
	});
}
function Ze(e) {
	return new E({
		code: "VGPU-PASS-CLEARSTENCIL-INVALID",
		message: `clearStencil ${e}`,
		fix: "Use an integer in [0, 0xFFFFFFFF] on a target whose depth format has a stencil aspect, e.g. depth: \"depth24plus-stencil8\"; the value is masked to the stencil aspect's bit width.",
		where: "Frame.pass"
	});
}
function Qe() {
	return new E({
		code: "VGPU-PASS-PRESERVE-CLEARSTENCIL",
		message: "clear:false preserves stencil; clearStencil cannot apply.",
		fix: "Remove clearStencil, or let the pass clear.",
		where: "Frame.pass"
	});
}
function A(e, t, n = "Frame.pass") {
	return new E({
		code: "VGPU-PASS-DEPTH-READONLY",
		message: `depthReadOnly ${e}`,
		fix: t,
		where: n
	});
}
function $e() {
	return new E({
		code: "VGPU-PASS-DEPTH-READONLY-MSAA",
		message: "depthReadOnly cannot read an MSAA target's depth: multisampled depth is stored with storeOp \"discard\", so a read-only pass tests against discarded contents.",
		fix: "Use a non-MSAA target for read-only depth, or drop depthReadOnly and let the pass own its depth.",
		where: "Frame.pass"
	});
}
function et(e, t, n = "timer") {
	return new E({
		code: "VGPU-TIMER-INVALID",
		message: `Invalid timer use: ${e}`,
		fix: t,
		where: n
	});
}
function tt(e, t, n = "visibility") {
	return new E({
		code: "VGPU-VIS-INVALID",
		message: `Invalid visibility use: ${e}`,
		fix: t,
		where: n
	});
}
function nt() {
	return new E({
		code: "VGPU-QUERY-NO-VISIBILITY",
		message: "occlusion() needs the pass to be opened with a visibility instance; the render pass has no occlusionQuerySet to write into.",
		fix: "Open the pass with f.pass({ target, visibility: vis }, ...) using the visibility(gpu) instance that created the query handle.",
		where: "FramePass.occlusion"
	});
}
function rt() {
	return new E({
		code: "VGPU-QUERY-NESTED",
		message: "occlusion() cannot nest inside an active occlusion() body; WebGPU allows one active occlusion query per pass at a time.",
		fix: "Encode each occlusion scope sequentially: p.occlusion(a, ...); p.occlusion(b, ...).",
		where: "FramePass.occlusion"
	});
}
function it(e = "Frame.pass") {
	return new E({
		code: "VGPU-TARGET-REQUIRED",
		message: "Target required. Fix: pass surface(gpu, canvas) or target(gpu, { size }) as { target }.",
		where: e
	});
}
function at(e, t, n, r) {
	return new E({
		code: e,
		message: `${e}: ${n}`,
		fix: r,
		where: t
	});
}
function ot(e, t) {
	return at("VGPU-MESH-RANGE-INVALID", e, t, "Use index ranges for indexed geometries, vertex ranges otherwise, within geometry counts.");
}
function st(e) {
	return new E({
		code: "VGPU-PIPELINE-LAYOUT-GAP",
		message: `Pipeline bind group ${e} is missing.`,
		fix: "Use consecutive @group() indices starting at 0.",
		where: "pipeline layout"
	});
}
function ct(e, t, n) {
	return new E({
		code: "VGPU-COMPILE-FAILED",
		message: "WebGPU pipeline compilation failed.",
		fix: "Check WGSL, vertex layouts, and target signature.",
		where: e,
		cause: t,
		detail: n ? { signature: n } : void 0
	});
}
function lt(e) {
	return new E({
		code: "VGPU-COMPILE-DISPOSED",
		message: "GPU disposed during pipeline compilation.",
		where: e
	});
}
function ut(e, t) {
	return new E({
		code: "VGPU-COMPILE-SIGNATURE-INVALID",
		message: `Invalid TargetSignature: ${t}`,
		fix: "Pass { colors, depth?, sampleCount?:1|4 } or a Target.",
		where: e
	});
}
function dt(e) {
	return new E({
		code: "VGPU-TARGET-DEPTH-STENCIL-ONLY",
		message: `depth received '${e}'; stencil-only depth targets are not supported yet.`,
		fix: "Use a format with a depth aspect such as \"depth24plus\" or \"depth24plus-stencil8\".",
		where: "target"
	});
}
function ft() {
	return new E({
		code: "VGPU-TARGET-SIZE-REQUIRED",
		message: "Target size required. Fix: target(gpu, { size: [w,h] }); update surface-derived targets in onResize.",
		where: "target"
	});
}
function pt(e) {
	return new E({
		code: "VGPU-SURFACE-NOT-IN-FRAME",
		message: "Surface targets are only available inside frame(gpu).",
		fix: "surface passes must run inside frame(gpu, ...); precompile against an offscreen target(gpu, ...) instead",
		where: e
	});
}
function mt() {
	return new E({
		code: "VGPU-SURFACE-CONTEXT",
		message: "Canvas WebGPU context failed. Fix: check navigator.gpu and remove any existing 2d/webgl context.",
		where: "surface"
	});
}
function ht(e) {
	return new E({
		code: "VGPU-SURFACE-DUPLICATE",
		message: `Canvas already has surface${e ? ` '${e}'` : ""}. Fix: reuse or dispose it.`,
		where: "surface"
	});
}
function gt(e) {
	return new E({
		code: "VGPU-SURFACE-DISPOSED",
		message: `Surface '${e ?? "surface"}' is disposed. Fix: call surface(gpu, canvas).`,
		where: "surface"
	});
}
function _t() {
	return new E({
		code: "VGPU-SURFACE-AUTORESIZE-UNSUPPORTED",
		message: "autoResize needs clientWidth. Fix: call surface.resize([w,h]) for OffscreenCanvas; onResize still fires.",
		where: "surface"
	});
}
function vt(e) {
	return new E({
		code: "VGPU-SURFACE-RESIZE-REENTRANT",
		message: `Cannot resize this surface${e ? ` '${e}'` : ""} in onResize. Fix: resize derived targets only.`,
		where: "surface.resize"
	});
}
function yt(e) {
	return new E({
		code: "VGPU-CLEAR-COLOR-INVALID",
		message: `Invalid ${e}: expected four finite numbers.`,
		fix: "Assign [r, g, b, a] or a GPUColor object ({ r, g, b, a }).",
		where: e
	});
}
function bt() {
	return new E({
		code: "VGPU-FRAME-REENTRANT",
		message: "Nested frame(gpu) is invalid. Fix: queue work for the next frame.",
		where: "frame"
	});
}
function xt(e) {
	return new E({
		code: "VGPU-FRAME-CANCELED",
		message: "the frame was canceled; its command encoder was dropped and nothing more can be encoded or submitted on it.",
		fix: "Open a new frame(gpu) for further work; cancel() is the last operation on a frame.",
		where: e
	});
}
function St(e) {
	return new E({
		code: "VGPU-FRAME-PASS-ACTIVE",
		message: "the frame cannot be canceled while a pass callback is active.",
		fix: "Return from the frame.pass(...) callback first, then call frame.cancel(); this keeps pass descriptor resources alive until the pass is closed.",
		where: e
	});
}
function Ct(e) {
	return new E({
		code: "VGPU-FRAME-SUBMITTED",
		message: "the frame was already submitted; submitted GPU work cannot be canceled.",
		fix: "Call cancel() only on a frame you decided not to submit; the frame you did submit needs no cleanup.",
		where: e
	});
}
function j(e, t, n) {
	return new E({
		code: "VGPU-R1-BINDING-INCOMPATIBLE-RESOURCE",
		message: `binding \`${e.name}\` @group(${e.group}) @binding(${e.binding}) needs ${t}.`,
		fix: n,
		where: "set"
	});
}
function M(e, t, n) {
	return new E({
		code: "VGPU-RING1-UNSUPPORTED",
		message: t,
		fix: n,
		where: e
	});
}
function wt(e) {
	return Tt(e) && e.version !== 1 ? new E({
		code: "VGPU-SHADER-SOURCE-INVALID",
		message: `VGPU-SHADER-SOURCE-INVALID: unsupported ShaderSource v${String(e.version)}; expected v1. Fix: update vgpu or regenerate it.`,
		where: "shader source"
	}) : new E({
		code: "VGPU-SHADER-SOURCE-INVALID",
		message: `VGPU-SHADER-SOURCE-INVALID: expected WGSL or { version, wgsl }, got ${Et(e)}. Fix: configure @vgpu/wgsl loader-vite or loader-webpack.`,
		where: "shader source"
	});
}
function Tt(e) {
	return typeof e == "object" && !!e && "version" in e;
}
function Et(e) {
	if (typeof e != "object" || !e) return typeof e;
	try {
		let t = JSON.stringify(e);
		return t.length > 80 ? `${t.slice(0, 77)}...` : t;
	} catch {
		return "object";
	}
}
function Dt(e, t) {
	switch (t.kind) {
		case "sampler": return `${e}.set({${t.name}:sampler(gpu)})`;
		case "texture": return `${e}.set({${t.name}:scene.color})`;
		case "buffer": return t.addressSpace === "uniform" ? `${e}.set({${t.name}:{ /* values */ }})` : `${e}.set({${t.name}:buffer})`;
		default: return `${e}.set({${t.name}:resource})`;
	}
}
//#endregion
//#region node_modules/vgpu/dist/kernel.js
var Ot = [
	"scheduler",
	"resource",
	"service"
];
function kt(e) {
	return { name: e };
}
var At = /* @__PURE__ */ new WeakMap();
function jt(e) {
	let t = At.get(e);
	if (!t) throw new E({
		code: "VGPU-GPU-FOREIGN",
		message: "This object was not created by init(); it has no vgpu kernel.",
		fix: "Pass the gpu returned by init() from vgpu, vgpu/node or vgpu/mock.",
		where: "gpu"
	});
	return t;
}
var Mt = class {
	device;
	#e = /* @__PURE__ */ new Map();
	#t = new Map(Ot.map((e) => [e, /* @__PURE__ */ new Set()]));
	#n = /* @__PURE__ */ new Set();
	#r = /* @__PURE__ */ new Set();
	#i = /* @__PURE__ */ new Set();
	#a = !1;
	constructor(e) {
		this.device = e;
	}
	get disposed() {
		return this.#a;
	}
	service(e, t) {
		let n = this.#e.get(e);
		if (n !== void 0) return n;
		let r = t(this);
		return this.#e.set(e, r), r;
	}
	peekService(e) {
		return this.#e.get(e);
	}
	own(e, t) {
		let n = this.#t.get(e);
		return n.add(t), () => {
			n.delete(t);
		};
	}
	addErrorListener(e) {
		return this.#n.add(e), () => {
			this.#n.delete(e);
		};
	}
	reportError(e) {
		if (this.#a) return Promise.resolve();
		let t = Promise.resolve().then(() => {
			let t = [...this.#n];
			if (!t.length) {
				console.error(e);
				return;
			}
			for (let n of t) try {
				n(e);
			} catch (e) {
				console.error(e);
			}
		});
		return this.trackDelivery(t);
	}
	trackDelivery(e) {
		let t = Promise.resolve(e).then(() => void 0, (e) => {
			console.error(e);
		});
		return this.#r.add(t), t.finally(() => this.#r.delete(t)), t;
	}
	registerSettledSource(e) {
		return this.#i.add(e), () => {
			this.#i.delete(e);
		};
	}
	async settled() {
		let e = [...this.#r, ...[...this.#i].flatMap((e) => e())];
		await Promise.allSettled(e);
	}
	dispose() {
		if (!this.#a) {
			this.#a = !0;
			for (let e of Ot) {
				let t = this.#t.get(e);
				for (let e of [...t]) e();
				t.clear();
			}
			this.#e.clear(), this.#i.clear(), this.#n.clear(), this.device.dispose();
		}
	}
};
function Nt(e) {
	let t = new Mt(e), n = {
		device: e,
		gpu: e.gpu,
		get disposed() {
			return t.disposed;
		},
		onError: (e) => t.addErrorListener(e),
		settled: () => t.settled(),
		dispose: () => {
			t.dispose();
		}
	};
	return At.set(n, t), n;
}
async function Pt(e, t = {}, n) {
	return Nt(await Ft(e, t, n));
}
async function Ft(e, t, n) {
	if (t.adapter || n) return (t.adapter ?? n()).requestDevice(t);
	if (e === "browser") return It(t);
	throw M("init", `init(${e}) requires adapterFactory.`);
}
async function It(e) {
	let t = await globalThis.navigator.gpu?.requestAdapter({ powerPreference: e.powerPreference });
	if (!t) throw M("init", "navigator.gpu.requestAdapter() returned null.");
	return r(t.features, e.requiredFeatures), new he(await t.requestDevice({
		requiredFeatures: e.requiredFeatures,
		requiredLimits: e.requiredLimits
	}), t.info ?? null);
}
//#endregion
//#region node_modules/vgpu/dist/lifecycle.js
function N(e, t) {
	e.assertUsable(t);
}
function Lt(e, t) {
	e.assertUsable(t);
}
//#endregion
//#region node_modules/vgpu/dist/draw-protocols.js
var Rt = Symbol("vgpu.bindingResource");
function zt(e) {
	return typeof (typeof e == "object" && e ? e[Rt] : void 0) == "function" ? e : void 0;
}
var Bt = Symbol("vgpu.geometry.layoutResolver");
//#endregion
//#region node_modules/vgpu/dist/live-kernel.js
function Vt(e, t) {
	let n = jt(e);
	if (n.disposed) throw Ht(t);
	return n;
}
function Ht(e) {
	return new E({
		code: "VGPU-GPU-DISPOSED",
		message: `${e}() ran after gpu.dispose(); the device and everything it owned are gone.`,
		fix: "Create resources before disposing the gpu, or init() a new one.",
		where: e
	});
}
function Ut(e, t, n, r) {
	let i = e.own("resource", () => n(t));
	return r?.(i), t;
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/errors.js
var Wt = class extends Error {
	code;
	line;
	column;
	severity;
	metadata;
	relatedDiagnostics;
	fix;
	where;
	cause;
	constructor(e, t, n = 1, r = 1, i = "error") {
		super(t), this.name = "VGPUError", this.code = e, this.line = n, this.column = r, this.severity = i;
	}
};
function Gt(e, t, n = {}) {
	let r = new Wt(e, t, n.line ?? 1, n.column ?? 1, n.severity ?? "error");
	return n.fix !== void 0 && (r.fix = n.fix), n.where !== void 0 && (r.where = n.where), n.cause !== void 0 && (r.cause = n.cause), n.metadata !== void 0 && (r.metadata = n.metadata), r;
}
function P(e, t, n = 1, r = 1) {
	return new Wt(e, t, n, r);
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/parser.js
var Kt = /* @__PURE__ */ new Set([
	"fn",
	"struct",
	"const",
	"alias",
	"var",
	"override"
]);
function qt(e) {
	let t = [], n = [], r = [], i = 0, a = !1, o = 0;
	for (; i < e.length;) {
		let s = e[i];
		if (s.text === "{") {
			o++, i++;
			continue;
		}
		if (s.text === "}") {
			o = Math.max(0, o - 1), i++;
			continue;
		}
		if ($t(s)) {
			i++;
			continue;
		}
		if (o > 0) {
			i++;
			continue;
		}
		if (s.text === "import") {
			if (a) throw P("VGPU-WGSL-IMP-ORDER", "Imports must precede declarations", s.line, s.column);
			let [n, r] = Jt(e, i);
			t.push(n), i = r;
			continue;
		}
		if (s.text === "export" && e[i + 1]?.text === "{") throw P("VGPU-WGSL-EXP-REEXPORT-CYCLE", "Re-export cycles are not supported", s.line, s.column);
		if (s.text === "@" && e[i + 2]?.text === "export" && e[i + 3]?.text === "@") throw P("VGPU-WGSL-EXP-NOTDECL", "Repeated export attributes", s.line, s.column);
		let c = s.text === "export" || s.text === "@" && e[i + 2]?.text === "export", l = c ? Yt(e, s.text === "export" ? i + 1 : i + 3) : i, u = e[l];
		if (u && Kt.has(u.text)) {
			let t = Xt(e, l);
			n.push({
				name: t,
				localName: t,
				kind: u.text
			}), c && r.push({
				name: t,
				localName: t,
				kind: u.text
			}), a = !0;
		}
		i++;
	}
	return {
		imports: t,
		exports: r,
		locals: n
	};
}
function Jt(e, t) {
	let n = t + 1, r = [];
	if (e[n]?.text === "{") {
		for (n++; e[n] && e[n].text !== "}";) {
			if ($t(e[n])) {
				n++;
				continue;
			}
			let t = Qt(e[n]), i = t;
			n++, e[n]?.text === "as" && (i = Qt(e[n + 1]), n += 2), r.push({
				imported: t,
				local: i
			}), e[n]?.text === "," && n++;
		}
		n++, Zt(e[n], "from"), n++;
	} else if (e[n]?.text === "*") Zt(e[n + 1], "as"), r.push({
		imported: "*",
		local: Qt(e[n + 2]),
		namespace: !0
	}), n += 3, Zt(e[n], "from"), n++;
	else if (e[n]?.kind === "string") throw P("VGPU-WGSL-IMP-SIDEEFFECT", "Side-effect imports are not supported", e[n].line, e[n].column);
	else throw P("VGPU-WGSL-IMP-DEFAULT", "Default imports are not supported", e[n]?.line, e[n]?.column);
	let i = e[n];
	if (i?.kind !== "string") throw P("VGPU-WGSL-RES-NOTFOUND", "Import path must be a string", i?.line, i?.column);
	let a = i.text.slice(1, -1);
	return n++, e[n]?.text === ";" && n++, [{
		from: a,
		bindings: r,
		start: e[t].start,
		end: e[n - 1].end
	}, n];
}
function Yt(e, t) {
	for (; e[t]?.text === "@";) {
		if (t += 2, e[t]?.text === "(") for (; e[t] && e[t].text !== ")";) t++;
		e[t]?.text === ")" && t++;
	}
	return t;
}
function Xt(e, t) {
	let n = t + 1;
	if (e[t]?.text === "var" && e[n]?.text === "<") for (; e[n] && e[n].text !== ">";) n++;
	for (; n < e.length; n++) if (e[n].kind === "ident") return e[n].text;
	throw P("VGPU-WGSL-EXP-NOTDECL", "Exported declaration has no name", e[t]?.line, e[t]?.column);
}
function Zt(e, t) {
	if (e?.text !== t) throw P("VGPU-WGSL-IMP-DEFAULT", `Expected ${t}`, e?.line, e?.column);
}
function Qt(e) {
	if (e?.kind !== "ident") throw P("VGPU-WGSL-IMP-DEFAULT", "Expected identifier", e?.line, e?.column);
	return e.text;
}
function $t(e) {
	return e.kind === "lineComment" || e.kind === "blockComment";
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-bind-layout.js
function en(e, t) {
	return t === "uniform" || t === "storage" ? "buffer" : e.kind === "sampler" ? "sampler" : e.kind === "texture" ? e.textureKind === "texture_external" ? "externalTexture" : "texture" : "unknown";
}
function tn(e, t, n, r, i) {
	if (e === "buffer") return nn(t, n, i);
	if (r.kind === "sampler") return rn(r);
	if (r.kind === "texture") return r.textureKind === "texture_external" ? {
		kind: "externalTexture",
		externalTexture: {}
	} : r.textureKind.startsWith("texture_storage_") ? an(r) : on(r);
}
function nn(e, t, n) {
	return {
		kind: "buffer",
		buffer: {
			type: e === "uniform" ? "uniform" : t === "read" ? "read-only-storage" : "storage",
			hasDynamicOffset: !1,
			minBindingSize: n?.size
		}
	};
}
function rn(e) {
	return {
		kind: "sampler",
		sampler: { type: e.comparison ? "comparison" : "filtering" }
	};
}
function an(e) {
	return {
		kind: "storageTexture",
		storageTexture: {
			access: ln(e.access),
			format: e.texelFormat ?? "rgba8unorm",
			viewDimension: cn(e.dimension)
		}
	};
}
function on(e) {
	return {
		kind: "texture",
		texture: {
			sampleType: sn(e),
			viewDimension: cn(e.dimension),
			multisampled: e.dimension === "multisampled_2d" || e.dimension === "depth_multisampled_2d"
		}
	};
}
function sn(e) {
	if (e.textureKind.startsWith("texture_depth_")) return "depth";
	let t = e.sampleType;
	return t?.kind === "scalar" && t.name === "i32" ? "sint" : t?.kind === "scalar" && t.name === "u32" ? "uint" : "unfilterable-float";
}
function cn(e) {
	switch (e) {
		case "1d": return "1d";
		case "2d_array":
		case "depth_2d_array": return "2d-array";
		case "cube":
		case "depth_cube": return "cube";
		case "cube_array":
		case "depth_cube_array": return "cube-array";
		case "3d": return "3d";
		default: return "2d";
	}
}
function ln(e) {
	return e === "read" ? "read-only" : e === "read_write" ? "read-write" : "write-only";
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/xxh64.js
var F = (1n << 64n) - 1n, I = 11400714785074694791n, un = 14029467366897019727n, dn = 1609587929392839161n, fn = 9650029242287828579n, pn = 2870177450012600261n;
function mn(e, t = 0n) {
	let n = new TextEncoder().encode(e), r = 0, i;
	if (n.length >= 32) {
		let e = t + I + un, a = t + un, o = t, s = t - I, c = n.length - 32;
		do
			e = L(e, gn(n, r)), r += 8, a = L(a, gn(n, r)), r += 8, o = L(o, gn(n, r)), r += 8, s = L(s, gn(n, r)), r += 8;
		while (r <= c);
		i = R(e, 1n) + R(a, 7n) + R(o, 12n) + R(s, 18n), i = hn(i, e), i = hn(i, a), i = hn(i, o), i = hn(i, s);
	} else i = t + pn;
	for (i = i + BigInt(n.length) & F; r + 8 <= n.length;) i ^= L(0n, gn(n, r)), i = R(i, 27n) * I + fn & F, r += 8;
	for (r + 4 <= n.length && (i ^= _n(n, r) * I & F, i = R(i, 23n) * un + dn & F, r += 4); r < n.length;) i ^= BigInt(n[r]) * pn & F, i = R(i, 11n) * I & F, r++;
	return i ^= i >> 33n, i = i * un & F, i ^= i >> 29n, i = i * dn & F, i ^= i >> 32n, i.toString(16).padStart(16, "0");
}
function L(e, t) {
	return R(e + t * un & F, 31n) * I & F;
}
function hn(e, t) {
	return e ^= L(0n, t), e * I + fn & F;
}
function R(e, t) {
	return (e << t | e >> 64n - t) & F;
}
function gn(e, t) {
	let n = 0n;
	for (let r = 7; r >= 0; r--) n = (n << 8n) + BigInt(e[t + r]);
	return n;
}
function _n(e, t) {
	return BigInt(e[t]) | BigInt(e[t + 1]) << 8n | BigInt(e[t + 2]) << 16n | BigInt(e[t + 3]) << 24n;
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/mangler.js
function vn(e) {
	return mn(e);
}
function yn(e) {
	return vn(e).slice(0, 8);
}
function bn(e, t) {
	return `_vgsl_${yn(e)}__${t}`;
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-utils.js
function z(e, t) {
	let n = e.find((e) => e.name === t);
	if (!n) return;
	let r = n.args.map((e) => e.text).join(""), i = Number(r.replace(/[ui]$/, ""));
	return Number.isFinite(i) ? i : void 0;
}
function xn(e) {
	let t = [[]], n = 0, r = 0;
	for (let i of e) {
		if (i.text === "<" ? n++ : i.text === ">" ? n = Math.max(0, n - 1) : i.text === "(" ? r++ : i.text === ")" && (r = Math.max(0, r - 1)), i.text === "," && n === 0 && r === 0) {
			t.push([]);
			continue;
		}
		t[t.length - 1].push(i);
	}
	return t.map(Sn).filter((e) => e.length > 0);
}
function Sn(e) {
	let t = 0, n = e.length;
	for (; t < n && e[t].text === ",";) t++;
	for (; n > t && e[n - 1].text === ",";) n--;
	return e.slice(t, n);
}
function Cn(e) {
	if (e !== void 0 && wn(e)) return Number(e.replace(/[ui]$/, ""));
}
function wn(e) {
	return /^(0|[1-9][0-9]*)([ui])?$/.test(e);
}
function Tn(e) {
	if (e === "read" || e === "write" || e === "read_write") return e;
}
function En(e) {
	return [
		"f32",
		"f16",
		"i32",
		"u32",
		"bool"
	].find((t) => t === e);
}
function Dn(e) {
	return {
		kind: "scalar",
		name: e === "f" ? "f32" : e === "h" ? "f16" : e === "i" ? "i32" : "u32"
	};
}
function On(e) {
	return e === "f16" ? 2 : 4;
}
function B(e, t) {
	return Math.ceil(t / e) * e;
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-type-parser.js
function V(e) {
	let t = Sn(e);
	if (t.length === 0) throw P("VGPU-WGSL-REFLECT-TYPE", "Expected WGSL type");
	let n = t.map((e) => e.text).join(""), r = kn(n);
	if (r) return r;
	if (t[1]?.text === "<") {
		let e = t[0].text, n = An(e, xn(t.slice(2, -1)));
		if (n) return n;
	}
	return jn(n) || Mn(n);
}
function kn(e) {
	let t = En(e);
	if (t) return {
		kind: "scalar",
		name: t
	};
	let n = e.match(/^vec([234])([fiuh])$/);
	if (n) return {
		kind: "vector",
		width: Number(n[1]),
		element: Dn(n[2])
	};
	let r = e.match(/^mat([234])x([234])([fh])$/);
	if (r) {
		let e = r[3] === "h" ? {
			kind: "scalar",
			name: "f16"
		} : {
			kind: "scalar",
			name: "f32"
		};
		return {
			kind: "matrix",
			columns: Number(r[1]),
			rows: Number(r[2]),
			element: e
		};
	}
}
function An(e, t) {
	if (e === "array") {
		let e = t[1]?.map((e) => e.text).join(""), n = e === void 0 ? void 0 : Cn(e);
		return {
			kind: "array",
			element: V(t[0] ?? []),
			count: n,
			countExpression: e
		};
	}
	if (e === "atomic") return {
		kind: "atomic",
		element: V(t[0] ?? [])
	};
	if (e === "vec2" || e === "vec3" || e === "vec4") return {
		kind: "vector",
		width: Number(e.slice(3)),
		element: V(t[0] ?? [])
	};
	if (/^mat[234]x[234]$/.test(e)) return {
		kind: "matrix",
		columns: Number(e[3]),
		rows: Number(e[5]),
		element: V(t[0] ?? [])
	};
	if (e === "ptr") return {
		kind: "ptr",
		addressSpace: t[0]?.map((e) => e.text).join("") ?? "",
		element: V(t[1] ?? []),
		access: t[2]?.map((e) => e.text).join("")
	};
	if (e === "sampler") return {
		kind: "sampler",
		comparison: !1
	};
	if (e.startsWith("texture_storage_")) return {
		kind: "texture",
		textureKind: e,
		dimension: e.slice(16),
		texelFormat: t[0]?.map((e) => e.text).join(""),
		access: Tn(t[1]?.map((e) => e.text).join(""))
	};
	if (e.startsWith("texture_")) return {
		kind: "texture",
		textureKind: e,
		dimension: e.slice(8),
		sampleType: t[0] ? V(t[0]) : void 0
	};
}
function jn(e) {
	if (e === "sampler" || e === "sampler_comparison") return {
		kind: "sampler",
		comparison: e === "sampler_comparison"
	};
	if (e === "texture_external") return {
		kind: "texture",
		textureKind: e
	};
	if (e.startsWith("texture_depth_") || e.startsWith("texture_")) return {
		kind: "texture",
		textureKind: e,
		dimension: e.slice(8)
	};
}
function Mn(e) {
	return {
		kind: "identifier",
		name: e
	};
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-token-utils.js
function H(e) {
	if (e?.kind !== "ident" && e?.kind !== "keyword") throw P("VGPU-WGSL-REFLECT-PARSE", "Expected identifier", e?.line, e?.column);
	return e.text;
}
function U(e, t, n) {
	for (let r = t; r < e.length; r++) if (e[r].text === n) return r;
	throw P("VGPU-WGSL-REFLECT-PARSE", `Expected ${n}`, e[t]?.line, e[t]?.column);
}
function Nn(e, t, n, r) {
	for (let i = t; i < n; i++) if (e[i].text === r) return i;
}
function Pn(e, t, n) {
	let r = 0;
	for (let i = t; i < e.length; i++) if ((e[i].text === "{" || e[i].text === "(") && r++, (e[i].text === "}" || e[i].text === ")") && (r = Math.max(0, r - 1)), r === 0 && e[i].text === n) return i;
	return e.length;
}
function Fn(e, t) {
	let n = e[t].text, r = n === "(" ? ")" : n === "{" ? "}" : ">", i = 0;
	for (let a = t; a < e.length; a++) if (e[a].text === n && i++, e[a].text === r && (i--, i === 0)) return a;
	throw P("VGPU-WGSL-REFLECT-PARSE", `Unclosed ${n}`, e[t]?.line, e[t]?.column);
}
function In(e, t) {
	let n = [], r = t;
	for (; e[r]?.text === "@";) {
		let t = e[r], i = H(e[r + 1]);
		r += 2;
		let a = [];
		if (e[r]?.text === "(") {
			let t = Fn(e, r);
			a = e.slice(r + 1, t), r = t + 1;
		}
		n.push({
			name: i,
			args: a,
			token: t
		});
	}
	return [n, r];
}
function Ln(e) {
	switch (e.kind) {
		case "scalar": return e.name;
		case "identifier": return e.name;
		case "vector": return `vec${e.width}<${Ln(e.element)}>`;
		case "matrix": return `mat${e.columns}x${e.rows}<${Ln(e.element)}>`;
		case "array": return `array<${Ln(e.element)}${e.count === void 0 ? "" : `,${e.count}`}>`;
		default: return e.kind;
	}
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-entry-points.js
function Rn(e) {
	let t = e.find((e) => e.name === "workgroup_size");
	if (!t) return;
	let n = xn(t.args).map((e) => Number(e.map((e) => e.text).join("")));
	return [
		n[0] ?? 1,
		n[1] ?? 1,
		n[2] ?? 1
	];
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-vars.js
function zn(e, t) {
	if (e[t]?.text !== "<") return { after: t };
	let n = U(e, t, ">"), r = xn(e.slice(t + 1, n)).map((e) => e.map((e) => e.text).join(""));
	return {
		addressSpace: r[0],
		access: Tn(r[1]),
		after: n + 1
	};
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-declarations.js
function Bn(e) {
	let t = [], n = [], r = [], i = [], a = [], o = [], s = e.tokens.filter((e) => e.kind !== "lineComment" && e.kind !== "blockComment"), c = 0, l = 0;
	for (; c < s.length;) {
		let u = s[c];
		if (u.text === "{") {
			l++, c++;
			continue;
		}
		if (u.text === "}") {
			l = Math.max(0, l - 1), c++;
			continue;
		}
		if (l > 0) {
			c++;
			continue;
		}
		let d = c, [f, p] = In(s, c);
		c = p, s[c]?.text === "export" && c++;
		let m = s[c]?.text;
		if (m === "enable") {
			s[c + 1]?.kind === "ident" && o.push(s[c + 1].text), c = Pn(s, c, ";") + 1;
			continue;
		}
		if (m === "struct") {
			let n = Vn(e, s, c, f);
			n.item && t.push(n.item), c = n.next;
			continue;
		}
		if (m === "alias") {
			let t = Hn(e, s, c, f);
			t.item && n.push(t.item), c = t.next;
			continue;
		}
		if (m === "var") {
			let t = Un(e, s, c, f);
			t.item && r.push(t.item), c = t.next;
			continue;
		}
		if (m === "fn") {
			let t = Wn(e, s, c, f);
			t.item && i.push(t.item), c = t.next;
			continue;
		}
		if (m === "override") {
			let e = Kn(s, c, f);
			e.item && a.push(e.item), c = e.next;
			continue;
		}
		c = Math.max(d + 1, c + 1);
	}
	return {
		structs: t,
		aliases: n,
		vars: r,
		entries: i,
		overrides: a,
		features: o
	};
}
function Vn(e, t, n, r) {
	let i = H(t[n + 1]), a = U(t, n + 2, "{"), o = Fn(t, a);
	return {
		item: {
			name: i,
			originalName: i,
			mangledName: Jn(e, i, "struct"),
			members: qn(t.slice(a + 1, o)),
			path: e.path
		},
		next: o + 1
	};
}
function Hn(e, t, n, r) {
	let i = H(t[n + 1]), a = U(t, n + 2, "="), o = Pn(t, a + 1, ";");
	return {
		item: {
			name: i,
			originalName: i,
			mangledName: Jn(e, i, "alias"),
			target: V(t.slice(a + 1, o)),
			path: e.path
		},
		next: o + 1
	};
}
function Un(e, t, n, r) {
	let { addressSpace: i, access: a, after: o } = zn(t, n + 1), s = H(t[o]), c = U(t, o + 1, ":"), l = Pn(t, c + 1, ";");
	return {
		item: {
			path: e.path,
			name: s,
			mangledName: Yn(r) ? s : Jn(e, s, "var"),
			attrs: r,
			addressSpace: i,
			access: a,
			type: V(t.slice(c + 1, l))
		},
		next: l + 1
	};
}
function Wn(e, t, n, r) {
	let i = H(t[n + 1]), a = r.find((e) => e.name === "vertex" || e.name === "fragment" || e.name === "compute")?.name;
	if (!a) return {
		item: void 0,
		next: n + 1
	};
	let o = U(t, n + 2, "("), s = Fn(t, o);
	return {
		item: {
			name: i,
			mangledName: i,
			stage: a,
			workgroupSize: Rn(r),
			path: e.path,
			params: Gn(t.slice(o + 1, s))
		},
		next: s + 1
	};
}
function Gn(e) {
	let t = [], n = 0;
	for (; n < e.length;) {
		let [r, i] = In(e, n);
		if (n = i, !e[n] || e[n].text === ",") {
			n++;
			continue;
		}
		let a = H(e[n]), o = U(e, n + 1, ":"), s = o + 1, c = 0;
		for (; s < e.length && (e[s].text === "<" && c++, e[s].text === ">" && (c = Math.max(0, c - 1)), c !== 0 || e[s].text !== ",");) s++;
		t.push({
			name: a,
			attrs: r,
			type: V(e.slice(o + 1, s))
		}), n = s + 1;
	}
	return t;
}
function Kn(e, t, n) {
	let r = H(e[t + 1]), i = Pn(e, t + 1, ";"), a = Nn(e, t + 2, i, "=");
	return {
		item: {
			name: r,
			mangledName: r,
			id: z(n, "id"),
			defaultValue: a === void 0 ? void 0 : e.slice(a + 1, i).map((e) => e.text).join("")
		},
		next: i + 1
	};
}
function qn(e) {
	let t = [], n = 0;
	for (; n < e.length;) {
		let [r, i] = In(e, n);
		if (n = i, !e[n] || e[n].text === "," || e[n].text === ";") {
			n++;
			continue;
		}
		let a = H(e[n]), o = U(e, n + 1, ":"), s = o + 1, c = 0;
		for (; s < e.length && (e[s].text === "<" && c++, e[s].text === ">" && (c = Math.max(0, c - 1)), c !== 0 || e[s].text !== "," && e[s].text !== ";");) s++;
		t.push({
			name: a,
			attrs: r,
			type: V(e.slice(o + 1, s)),
			align: z(r, "align"),
			size: z(r, "size")
		}), n = s + 1;
	}
	return t;
}
function Jn(e, t, n) {
	return n === "override" ? t : bn(e.path, t);
}
function Yn(e) {
	return z(e, "group") !== void 0 || z(e, "binding") !== void 0;
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/diagnostics.js
var Xn = "literal length required for auto layout; use draw.group(n, bg) manual binding", Zn = "VGPUError: `bool` is not host-shareable in uniform/storage. Fix: use `u32` (0 | 1) → struct Params { enabled: u32 }", Qn = "use a manual group claim (`draw.group(n, bg)`)";
function $n(e = 1, t = 1) {
	return P("VGPU-WGSL-REFLECT-ARRAY-LENGTH", Xn, e, t);
}
function er(e = 1, t = 1) {
	return P("VGPU-WGSL-REFLECT-BOOL-HOST-SHAREABLE", Zn, e, t);
}
function tr(e, t, n = 1, r = 1) {
	return P("VGPU-WGSL-REFLECT-UNKNOWN-TYPE", `type '${e}' is unknown in ${t}; ${Qn}`, n, r);
}
function nr(e, t, n = 1, r = 1) {
	return P("VGPU-WGSL-REFLECT-NS-TYPE", `type '${e}' is a namespace-member import; use a named import or manual @group(1+) binding`, n, r);
}
function rr(e, t = 1, n = 1) {
	return P("VGPU-WGSL-REFLECT-NON-HOST-SHAREABLE", `Type ${e} is not host-shareable; ${Qn}`, t, n);
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-types.js
var ir = "naga-standard";
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-symbols.js
function ar(e, t, n) {
	let r = /* @__PURE__ */ new Map();
	for (let e of t) {
		let t = /* @__PURE__ */ new Map();
		for (let n of [...e.structs, ...e.aliases]) t.set(n.originalName, {
			path: n.path,
			name: n.originalName,
			mangledName: n.mangledName,
			kind: "members" in n ? "struct" : "alias"
		});
		r.set(e.structs[0]?.path ?? e.aliases[0]?.path ?? e.vars[0]?.path ?? "", t);
	}
	let i = new Map(e.map((e) => [e.path, r.get(e.path) ?? /* @__PURE__ */ new Map()])), a = /* @__PURE__ */ new Map();
	for (let t of e) {
		let r = new Map(i.get(t.path));
		for (let a of t.parsed.imports) or(t, a, r, e, i, n);
		a.set(t.path, r);
	}
	return a;
}
function or(e, t, n, r, i, a) {
	let o = ur(t, e.path, r, a), s = i.get(o);
	for (let e of t.bindings) {
		if (e.namespace) {
			n.set(e.local, {
				path: o,
				name: e.local,
				mangledName: e.local,
				kind: "namespace"
			});
			continue;
		}
		let t = s?.get(e.imported);
		t && n.set(e.local, t);
	}
}
function sr(e, t) {
	let n = /* @__PURE__ */ new Map(), r = /* @__PURE__ */ new Map(), i = /* @__PURE__ */ new Map(), a = {
		structs: n,
		aliases: r,
		byMangled: i
	};
	for (let o of e) {
		for (let e of o.structs) {
			let r = {
				name: e.name,
				mangledName: e.mangledName,
				members: e.members.map((n) => ({
					name: n.name,
					type: W(n.type, e.path, t, a),
					align: n.align,
					size: n.size
				}))
			};
			n.set(e.mangledName, r), i.set(e.mangledName, r);
		}
		for (let e of o.aliases) {
			let n = {
				name: e.name,
				mangledName: e.mangledName,
				target: W(e.target, e.path, t, a)
			};
			r.set(e.mangledName, n), i.set(e.mangledName, n);
		}
	}
	return {
		structs: n,
		aliases: r,
		byMangled: i
	};
}
function W(e, t, n, r) {
	switch (e.kind) {
		case "identifier": {
			let r = e.name.indexOf(".");
			if (r > 0) {
				let i = e.name.slice(0, r);
				if (n.get(t)?.get(i)?.kind === "namespace") throw nr(e.name, t);
			}
			let i = n.get(t)?.get(e.name);
			if (i?.kind === "namespace") throw nr(e.name, t);
			if (!i) throw tr(e.name, t);
			return {
				kind: "identifier",
				name: i.name,
				mangledName: i.mangledName
			};
		}
		case "array":
		case "atomic":
		case "vector":
		case "matrix":
		case "ptr": return {
			...e,
			element: W(e.element, t, n, r)
		};
		case "texture": return {
			...e,
			sampleType: e.sampleType ? W(e.sampleType, t, n, r) : void 0
		};
		default: return e;
	}
}
function cr(e, t) {
	if (!t || e.kind !== "identifier") return e;
	let n = t.aliases.get(e.mangledName ?? e.name);
	return n ? cr(n.target, t) : e;
}
function lr(e, t) {
	let n = cr(e, t);
	switch (n.kind) {
		case "array":
		case "atomic":
		case "vector":
		case "matrix":
		case "ptr": return {
			...n,
			element: lr(n.element, t)
		};
		case "texture": return {
			...n,
			sampleType: n.sampleType ? lr(n.sampleType, t) : void 0
		};
		default: return n;
	}
}
function ur(e, t, n, r) {
	let i = dr(e, t, r);
	if (i !== void 0 && n.some((e) => e.path === i)) return i;
	let a = e.from, o = t.slice(0, t.lastIndexOf("/") + 1), s = a.startsWith("/") ? a : fr(`${o}${a}`);
	return [a, s].find((e) => n.some((t) => t.path === e)) ?? i ?? s;
}
function dr(e, t, n) {
	if (n) try {
		return n(t, e);
	} catch {
		return;
	}
}
function fr(e) {
	let t = e.startsWith("/"), n = [];
	for (let t of e.split("/")) !t || t === "." || (t === ".." ? n.pop() : n.push(t));
	return `${t ? "/" : ""}${n.join("/")}`;
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-layout.js
function pr(e, t, n = Ln(e), r = n, i) {
	return mr(i ? lr(e, i) : e, t, n, r, i);
}
function mr(e, t, n, r, i) {
	switch (e.kind) {
		case "scalar": return hr(e, t, n, r);
		case "atomic": return gr(e, t, n, r);
		case "vector": return _r(e, t, n, r, i);
		case "matrix": return vr(e, t, n, r, i);
		case "array": return yr(e, t, n, r, i);
		case "identifier": return xr(e, t, n, r, i);
		default: throw rr(Ln(e));
	}
}
function hr(e, t, n, r) {
	let i = On(e.name);
	if (e.name === "bool") throw er();
	return {
		name: n,
		mangledName: r,
		addressSpace: t,
		layoutMode: ir,
		type: e,
		align: i,
		size: i
	};
}
function gr(e, t, n, r) {
	return {
		name: n,
		mangledName: r,
		addressSpace: t,
		layoutMode: ir,
		type: e,
		align: 4,
		size: 4
	};
}
function _r(e, t, n, r, i) {
	let a = pr(e.element, t, n, r, i).size ?? 4;
	return {
		name: n,
		mangledName: r,
		addressSpace: t,
		layoutMode: ir,
		type: e,
		align: e.width === 2 ? a * 2 : a * 4,
		size: a * e.width
	};
}
function vr(e, t, n, r, i) {
	let a = pr({
		kind: "vector",
		width: e.rows,
		element: e.element
	}, t, `${n}[]`, `${r}[]`, i), o = B(a.align, a.size ?? 0);
	return {
		name: n,
		mangledName: r,
		addressSpace: t,
		layoutMode: ir,
		type: e,
		align: a.align,
		size: o * e.columns,
		stride: o,
		element: a
	};
}
function yr(e, t, n, r, i) {
	br(e.countExpression);
	let a = pr(e.element, t, `${n}[]`, `${r}[]`, i), o = B(Er(e.element, t, i), a.size ?? 0);
	return {
		name: n,
		mangledName: r,
		addressSpace: t,
		layoutMode: ir,
		type: e,
		align: Er(e, t, i),
		size: e.count === void 0 ? void 0 : o * e.count,
		stride: o,
		element: a,
		runtimeSized: e.count === void 0
	};
}
function br(e) {
	if (e !== void 0 && !wn(e)) throw $n();
}
function xr(e, t, n, r, i) {
	if (!i) throw tr(e.name, "<unknown>");
	let a = i.structs.get(e.mangledName ?? e.name);
	if (!a) throw tr(e.name, "<unknown>");
	let o = [], s = 0, c = 1;
	for (let e of a.members) {
		let n = Sr(e, t, s, i);
		o.push(n.member), s = Cr(t, e.type, n.offset, n.member.size ?? 0, i), c = Math.max(c, n.member.align);
	}
	let l = Tr(t, c);
	return {
		name: n,
		mangledName: r,
		addressSpace: t,
		layoutMode: ir,
		type: e,
		align: l,
		size: B(l, s),
		members: o
	};
}
function Sr(e, t, n, r) {
	let i = pr(e.type, t, e.name, e.name, r), a = Math.max(Er(e.type, t, r), e.align ?? 1), o = Math.max(i.size ?? 0, e.size ?? 0), s = B(a, n);
	return {
		member: {
			name: e.name,
			offset: s,
			align: a,
			size: o,
			type: e.type,
			layout: i,
			explicitAlign: e.align,
			explicitSize: e.size
		},
		offset: s
	};
}
function Cr(e, t, n, r, i) {
	return n + (e === "uniform" && wr(t, i) ? B(16, r) : r);
}
function wr(e, t) {
	let n = cr(e, t);
	return n.kind === "identifier" && t.structs.has(n.mangledName ?? n.name);
}
function Tr(e, t) {
	return e === "uniform" ? B(16, t) : t;
}
function Er(e, t, n) {
	let r = n ? cr(e, n) : e, i = Or(r, t, n);
	return t === "uniform" && Dr(r, n) ? B(16, i) : i;
}
function Dr(e, t) {
	return e.kind === "array" || e.kind === "identifier" && !!t?.structs.get(e.mangledName ?? e.name);
}
function Or(e, t, n) {
	let r = n ? cr(e, n) : e;
	switch (r.kind) {
		case "scalar": return kr(r.name);
		case "atomic": return 4;
		case "vector": return r.width === 2 ? Or(r.element, t, n) * 2 : Or(r.element, t, n) * 4;
		case "matrix": return Or({
			kind: "vector",
			width: r.rows,
			element: r.element
		}, t, n);
		case "array": return Er(r.element, t, n);
		case "identifier": return Ar(r, t, n);
		default: throw rr(Ln(r));
	}
}
function kr(e) {
	if (e === "bool") throw er();
	return On(e);
}
function Ar(e, t, n) {
	let r = n?.structs.get(e.mangledName ?? e.name);
	if (!r) throw tr(e.name, "<unknown>");
	return Math.max(1, ...r.members.map((e) => Math.max(Er(e.type, t, n), e.align ?? 1)));
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/wgsl-identifiers.js
var jr = /* @__PURE__ */ new Set(/* @__PURE__ */ "alias.break.case.const.const_assert.continue.continuing.default.diagnostic.discard.else.enable.false.fn.for.if.let.loop.override.requires.return.struct.switch.true.var.while".split(".")), Mr = /* @__PURE__ */ new Set([
	"import",
	"export",
	"from",
	"as"
]), Nr = /* @__PURE__ */ new Set([...jr, ...Mr]), Pr = /* @__PURE__ */ new Set(/* @__PURE__ */ "NULL.Self.abstract.active.alignas.alignof.as.asm.asm_fragment.async.attribute.auto.await.become.cast.catch.class.co_await.co_return.co_yield.coherent.column_major.common.compile.compile_fragment.concept.const_cast.consteval.constexpr.constinit.crate.debugger.decltype.delete.demote.demote_to_helper.do.dynamic_cast.enum.explicit.export.extends.extern.external.fallthrough.filter.final.finally.friend.from.fxgroup.get.goto.groupshared.highp.impl.implements.import.inline.instanceof.interface.layout.lowp.macro.macro_rules.match.mediump.meta.mod.module.move.mut.mutable.namespace.new.nil.noexcept.noinline.nointerpolation.non_coherent.noncoherent.noperspective.null.nullptr.of.operator.package.packoffset.partition.pass.patch.pixelfragment.precise.precision.premerge.priv.protected.pub.public.readonly.ref.regardless.register.reinterpret_cast.require.resource.restrict.self.set.shared.sizeof.smooth.snorm.static.static_assert.static_cast.std.subroutine.super.target.template.this.thread_local.throw.trait.try.type.typedef.typeid.typename.typeof.union.unless.unorm.unsafe.unsized.use.using.varying.virtual.volatile.wgsl.where.with.writeonly.yield".split(".")), Fr = /* @__PURE__ */ new Set(["binding_array"]), Ir = /* @__PURE__ */ new Set(/* @__PURE__ */ "array.atomic.bool.f16.f32.i32.mat2x2.mat2x3.mat2x4.mat3x2.mat3x3.mat3x4.mat4x2.mat4x3.mat4x4.ptr.sampler.sampler_comparison.texture_1d.texture_2d.texture_2d_array.texture_3d.texture_cube.texture_cube_array.texture_depth_2d.texture_depth_2d_array.texture_depth_cube.texture_depth_cube_array.texture_depth_multisampled_2d.texture_external.texture_multisampled_2d.texture_storage_1d.texture_storage_2d.texture_storage_2d_array.texture_storage_3d.u32.vec2.vec2f.vec2h.vec2i.vec2u.vec3.vec3f.vec3h.vec3i.vec3u.vec4.vec4f.vec4h.vec4i.vec4u".split(".")), Lr = /* @__PURE__ */ new Set(/* @__PURE__ */ "abs.acos.acosh.all.any.arrayLength.asin.asinh.atan.atan2.atanh.ceil.clamp.cos.cosh.countLeadingZeros.countOneBits.countTrailingZeros.cross.degrees.determinant.distance.dot.dot4I8Packed.dot4U8Packed.dpdx.dpdxCoarse.dpdxFine.dpdy.dpdyCoarse.dpdyFine.exp.exp2.extractBits.faceForward.firstLeadingBit.firstTrailingBit.floor.fma.fract.frexp.fwidth.fwidthCoarse.fwidthFine.insertBits.inverseSqrt.ldexp.length.log.log2.max.min.mix.modf.normalize.pack2x16float.pack2x16snorm.pack2x16unorm.pack4x8snorm.pack4x8unorm.pack4xI8.pack4xU8.pack4xI8Clamp.pack4xU8Clamp.pow.quantizeToF16.radians.reflect.refract.reverseBits.round.saturate.select.sign.sin.sinh.smoothstep.sqrt.step.storageBarrier.tan.tanh.textureBarrier.textureDimensions.textureGather.textureGatherCompare.textureLoad.textureNumLayers.textureNumLevels.textureNumSamples.textureSample.textureSampleBaseClampToEdge.textureSampleBias.textureSampleCompare.textureSampleCompareLevel.textureSampleGrad.textureSampleLevel.textureStore.transpose.trunc.unpack2x16float.unpack2x16snorm.unpack2x16unorm.unpack4x8snorm.unpack4x8unorm.unpack4xI8.unpack4xU8.workgroupBarrier".split(".")), Rr = /* @__PURE__ */ new Set([
	"frag_depth",
	"front_facing",
	"global_invocation_id",
	"instance_index",
	"local_invocation_id",
	"local_invocation_index",
	"num_workgroups",
	"position",
	"sample_index",
	"sample_mask",
	"subgroup_invocation_id",
	"subgroup_size",
	"vertex_index",
	"workgroup_id"
]), zr = /* @__PURE__ */ new Set([
	"align",
	"binding",
	"blend_src",
	"builtin",
	"compute",
	"diagnostic",
	"fragment",
	"group",
	"id",
	"interpolate",
	"invariant",
	"location",
	"must_use",
	"size",
	"vertex",
	"workgroup_size"
]), Br = /* @__PURE__ */ new Set([
	"function",
	"private",
	"storage",
	"uniform",
	"workgroup"
]), Vr = /* @__PURE__ */ new Set([
	"read",
	"read_write",
	"write"
]), Hr = /* @__PURE__ */ new Set([
	"bgra8unorm",
	"r32float",
	"r32sint",
	"r32uint",
	"rg32float",
	"rg32sint",
	"rg32uint",
	"rgba16float",
	"rgba16sint",
	"rgba16uint",
	"rgba32float",
	"rgba32sint",
	"rgba32uint",
	"rgba8sint",
	"rgba8snorm",
	"rgba8uint",
	"rgba8unorm"
]);
[
	...Nr,
	...Pr,
	...Fr,
	...Ir,
	...Lr,
	...Rr,
	...zr,
	...Br,
	...Vr,
	...Hr
];
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/scanner.js
var Ur = "VGPU-WGSL-IDENT-NONASCII", Wr = "https://github.com/vercel-labs/vgpu/issues/294";
function Gr(e, t) {
	let n = [], r = 0, i = 1, a = 1, o = (t, r, i, a, o) => n.push({
		kind: t,
		text: e.slice(r, i),
		start: r,
		end: i,
		line: a,
		column: o
	}), s = () => {
		e[r] === "\n" ? (i++, a = 1) : a++, r++;
	};
	for (; r < e.length;) {
		let n = e[r];
		if (/\s/.test(n)) {
			s();
			continue;
		}
		let c = r, l = i, u = a;
		if (n === "/" && e[r + 1] === "/") {
			for (; r < e.length && e[r] !== "\n";) s();
			o("lineComment", c, r, l, u);
			continue;
		}
		if (n === "/" && e[r + 1] === "*") {
			let t = 0;
			for (; r < e.length;) {
				if (e[r] === "/" && e[r + 1] === "*") {
					t++, s(), s();
					continue;
				}
				if (e[r] === "*" && e[r + 1] === "/") {
					if (t--, s(), s(), t === 0) {
						o("blockComment", c, r, l, u);
						break;
					}
					continue;
				}
				s();
			}
			if (t !== 0) throw P("VGPU-WGSL-LEX-UNTERM-COMMENT", "Unterminated block comment", l, u);
			continue;
		}
		if (n === "\"" || n === "'") {
			let t = n;
			for (s(); r < e.length && e[r] !== t;) {
				if (e[r] === "\n") throw P("VGPU-WGSL-LEX-UNTERM-STRING", "Unterminated string", l, u);
				e[r] === "\\" && s(), s();
			}
			if (r >= e.length) throw P("VGPU-WGSL-LEX-UNTERM-STRING", "Unterminated string", l, u);
			s(), o("string", c, r, l, u);
			continue;
		}
		if (/[A-Za-z_]/.test(n)) {
			for (; r < e.length && /[A-Za-z0-9_]/.test(e[r]);) s();
			let t = e.slice(c, r);
			o(Nr.has(t) ? "keyword" : "ident", c, r, l, u);
			continue;
		}
		if (/[0-9]/.test(n) || n === "." && /[0-9]/.test(e[r + 1] ?? "")) {
			for (n === "." && s(); r < e.length;) {
				let t = e[r];
				if (/[A-Za-z0-9_.]/.test(t)) {
					s();
					continue;
				}
				if ((t === "+" || t === "-") && Jr(e[r - 1]) && /[0-9]/.test(e[r + 1] ?? "")) {
					s();
					continue;
				}
				break;
			}
			o("number", c, r, l, u);
			continue;
		}
		if (n.charCodeAt(0) > 127) throw Kr(e, r, i, a, t);
		s(), o("punct", c, r, l, u);
	}
	return n;
}
function Kr(e, t, n, r, i) {
	let a = t;
	for (; a > 0 && qr(e[a - 1]);) a--;
	let o = t + 1;
	for (; o < e.length && qr(e[o]);) o++;
	let s = e.slice(a, o), c = r - (t - a), l = Gt(Ur, `Non-ASCII identifier '${s}'${i === void 0 ? "" : ` in ${i}`} at line ${n} column ${c}; vgpu's WGSL pipeline supports ASCII identifiers only`, {
		fix: `Rename '${s}' using ASCII letters, digits and '_'. Unicode (XID) identifiers are tracked in ${Wr}`,
		line: n,
		column: c
	});
	return l.range = {
		file: i,
		start: {
			line: n,
			column: c
		}
	}, l;
}
function qr(e) {
	return e.charCodeAt(0) > 127 || /[A-Za-z0-9_]/.test(e);
}
function Jr(e) {
	return e === "e" || e === "E" || e === "p" || e === "P";
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/scope-walker.js
var Yr = /^_vgsl_[0-9a-f]{8,16}__[A-Za-z_][A-Za-z0-9_]*$/, Xr = /* @__PURE__ */ new Set([
	"fn",
	"struct",
	"const",
	"alias",
	"var",
	"override"
]);
function Zr(e) {
	return new Qr(e).analyze();
}
var Qr = class {
	tokens;
	scopes = [];
	declarations = [];
	references = [];
	functions = [];
	preserved = /* @__PURE__ */ new Map();
	symbolsByScope = /* @__PURE__ */ new Map();
	moduleFallbackReasons = [];
	pendingSymbols = [];
	moduleScopeId;
	constructor(e) {
		this.tokens = e, this.moduleScopeId = this.createScope("module", void 0, void 0, 0);
	}
	analyze() {
		this.collectTopLevel();
		for (let e of this.functions) this.walkFunction(e);
		return {
			tokens: this.tokens,
			scopes: this.scopes,
			declarations: this.declarations,
			references: this.references,
			functions: this.functions,
			preservedTokens: [...this.preserved.entries()].map(([e, t]) => ({
				tokenIndex: e,
				reason: t
			})),
			fallback: {
				wholeModule: this.moduleFallbackReasons.length > 0,
				reasons: this.moduleFallbackReasons
			}
		};
	}
	collectTopLevel() {
		let e = 0;
		for (let t = 0; t < this.tokens.length; t++) {
			let n = this.tokens[t];
			if (!G(n)) {
				if (n.text === "{") {
					e++;
					continue;
				}
				if (n.text === "}") {
					e--, e < 0 && (this.moduleFallback("unmatched top-level closing brace", t), e = 0);
					continue;
				}
				if (e === 0) {
					if (n.text === "@") {
						t = this.preserveAttribute(t);
						continue;
					}
					if (n.text === "enable" || n.text === "requires" || n.text === "diagnostic" || n.text === "const_assert") {
						t = this.preserveStatement(t, "directive");
						continue;
					}
					if (n.text !== "export") {
						if (n.text === "struct") {
							t = this.collectStruct(t);
							continue;
						}
						if (n.text === "fn") {
							t = this.collectFunction(t);
							continue;
						}
						if (n.text === "const" || n.text === "alias" || n.text === "var" || n.text === "override") {
							t = this.preserveGlobalDeclaration(t);
							continue;
						}
						n.kind === "keyword" && !Xr.has(n.text) && this.moduleFallback(`unexpected top-level keyword '${n.text}'`, t);
					}
				}
			}
		}
		e !== 0 && this.moduleFallback("unclosed top-level brace", this.tokens.length - 1), this.scopes[this.moduleScopeId].endToken = Math.max(0, this.tokens.length - 1);
	}
	collectStruct(e) {
		let t = this.nextSig(e);
		if (t === void 0 || this.tokens[t]?.kind !== "ident") return this.moduleFallback("struct without name", e), e;
		this.preserveToken(t, "global");
		let n = this.nextSig(t);
		if (n === void 0 || this.tokens[n]?.text !== "{") return this.moduleFallback("struct without body", e), t;
		let r = this.findMatching(n, "{", "}");
		if (r === void 0) return this.moduleFallback("unclosed struct body", n), n;
		for (let e = n; e <= r; e++) this.tokens[e]?.kind === "ident" && this.preserveToken(e, "struct");
		return r;
	}
	collectFunction(e) {
		let t = this.nextSig(e);
		if (t === void 0 || this.tokens[t]?.kind !== "ident") return this.moduleFallback("function without name", e), e;
		let n = this.tokens[t].text, r = Yr.test(n) && !this.hasEntryAttributeBefore(e);
		this.addDeclaration(n, "function", t, this.moduleScopeId, void 0, r), r || this.preserveToken(t, "global");
		let i = this.nextSig(t);
		if (i === void 0 || this.tokens[i]?.text !== "(") return this.moduleFallback("function without parameter list", t), t;
		let a = this.findMatching(i, "(", ")");
		if (a === void 0) return this.moduleFallback("unclosed function parameter list", i), i;
		let o = this.findNextText(a + 1, "{");
		if (o === void 0) return this.moduleFallback("function without body", a), a;
		this.preserveFunctionSignatureTail(a + 1, o);
		let s = this.findMatching(o, "{", "}");
		if (s === void 0) return this.moduleFallback("unclosed function body", o), o;
		let c = this.createScope("function", this.moduleScopeId, this.functions.length, i);
		return this.functions.push({
			id: this.functions.length,
			name: n,
			nameTokenIndex: t,
			scopeId: c,
			bodyStartToken: o,
			bodyEndToken: s,
			skipped: !1,
			fallbackReasons: []
		}), this.collectParams(i, a, c, this.functions.length - 1), this.scopes[c].endToken = s, s;
	}
	collectParams(e, t, n, r) {
		for (let i = e + 1; i < t; i++) {
			let e = this.tokens[i];
			if (!G(e)) {
				if (e.text === "@") {
					i = this.preserveAttribute(i);
					continue;
				}
				if (e.kind === "ident" && this.nextSig(i) !== void 0 && this.tokens[this.nextSig(i)]?.text === ":") {
					this.addDeclaration(e.text, "param", i, n, r, !0);
					let a = this.nextSig(i);
					i = this.preserveTypeFrom(a + 1, [",", ")"], t);
				}
			}
		}
	}
	preserveFunctionSignatureTail(e, t) {
		for (let n = e; n < t; n++) {
			let e = this.tokens[n];
			if (!G(e)) {
				if (e.text === "@") {
					n = this.preserveAttribute(n);
					continue;
				}
				e.kind === "ident" && this.preserveToken(n, "type");
			}
		}
	}
	preserveGlobalDeclaration(e) {
		let t = e + 1;
		if (this.tokens[e]?.text === "var") {
			let n = this.nextSig(e);
			if (n !== void 0 && this.tokens[n]?.text === "<") {
				let e = this.findMatching(n, "<", ">");
				if (e === void 0) return this.moduleFallback("unparseable top-level var template", n), n;
				this.preserveRange(n, e, "type"), t = e + 1;
			}
		}
		let n = this.findNextIdent(t);
		n !== void 0 && (this.preserveToken(n, "global"), this.addDeclaration(this.tokens[n].text, "global", n, this.moduleScopeId, void 0, !1));
		let r = this.findStatementEnd(e);
		for (let t = e; t <= r; t++) this.tokens[t]?.kind === "ident" && this.preserveToken(t, "global");
		return r;
	}
	walkFunction(e) {
		let t = [this.moduleScopeId, e.scopeId], n = [], r = (n, r) => {
			let i = this.createScope(n, t[t.length - 1], e.id, r);
			return t.push(i), i;
		}, i = (n) => {
			if (t.length <= 2) {
				this.functionFallback(e, "scope frame underflow", n);
				return;
			}
			let r = t.pop();
			return this.scopes[r].endToken = n, r;
		};
		r("block", e.bodyStartToken);
		let a = 1;
		for (let o = e.bodyStartToken + 1; o < e.bodyEndToken; o++) {
			this.activatePendingSymbols(o);
			let s = this.tokens[o];
			if (G(s)) continue;
			if (s.text === "@") {
				o = this.preserveAttribute(o);
				continue;
			}
			if (s.text === ".") {
				let e = this.nextSig(o);
				e !== void 0 && this.tokens[e]?.kind === "ident" && this.preserveToken(e, "member");
				continue;
			}
			if (s.text === "enable" || s.text === "requires" || s.text === "diagnostic") {
				o = this.preserveStatement(o, "directive");
				continue;
			}
			if (s.text === "for") {
				let t = r("for-init", o), i = this.nextSig(o);
				(i === void 0 || this.tokens[i]?.text !== "(") && this.functionFallback(e, "for without parenthesized header", o), n.push({
					scopeId: t,
					headerDepth: 0,
					awaitingBody: !1
				});
				continue;
			}
			let c = n[n.length - 1];
			if (c && c.bodyDepth === void 0 && (s.text === "(" && c.headerDepth++, s.text === ")" && (c.headerDepth--, c.headerDepth <= 0 && (c.awaitingBody = !0))), s.text === "{") {
				a++;
				let e = $r(n, (e) => e.awaitingBody && e.bodyDepth === void 0);
				e && (e.bodyDepth = a), r("block", o);
				continue;
			}
			if (s.text === "}") {
				let t = a;
				for (i(o), a--; n.length > 0 && n[n.length - 1].bodyDepth === t;) i(o), n.pop();
				a < 0 && this.functionFallback(e, "unmatched closing brace", o);
				continue;
			}
			if (s.text === ":") {
				o = this.preserveTypeFrom(o + 1, [
					"=",
					";",
					",",
					")",
					"{"
				], e.bodyEndToken);
				continue;
			}
			if (s.text === "-" && this.tokens[this.nextSig(o) ?? -1]?.text === ">") {
				o = this.preserveTypeFrom((this.nextSig(o) ?? o) + 1, ["{"], e.bodyEndToken);
				continue;
			}
			if (s.text === "let" || s.text === "const" || s.text === "var") {
				o = this.collectLocalDeclaration(o, t[t.length - 1], e);
				continue;
			}
			if (s.kind === "ident" && !this.preserved.has(o)) {
				let n = this.resolve(s.text, t);
				n === void 0 ? this.preserveToken(o, "unknown") : this.references.push({
					name: s.text,
					tokenIndex: o,
					declarationId: n,
					scopeId: t[t.length - 1],
					functionId: e.id
				});
			}
		}
		for (; t.length > 2;) i(e.bodyEndToken);
	}
	collectLocalDeclaration(e, t, n) {
		let r = this.tokens[e].text, i = e + 1;
		if (r === "var") {
			let t = this.nextSig(e);
			if (t !== void 0 && this.tokens[t]?.text === "<") {
				let e = this.findMatching(t, "<", ">");
				if (e === void 0) return this.functionFallback(n, "unparseable var template", t), t;
				this.preserveRange(t, e, "type"), i = e + 1;
			}
		}
		let a = this.findNextIdent(i);
		if (a === void 0 || a >= n.bodyEndToken) return this.functionFallback(n, `${r} without identifier`, e), e;
		this.addDeclaration(this.tokens[a].text, r, a, t, n.id, !0, this.findStatementEnd(e));
		let o = this.nextSig(a);
		return o !== void 0 && this.tokens[o]?.text === ":" ? this.preserveTypeFrom(o + 1, [
			"=",
			";",
			",",
			")"
		], n.bodyEndToken) : a;
	}
	addDeclaration(e, t, n, r, i, a, o) {
		let s = this.declarations.length;
		return this.declarations.push({
			id: s,
			name: e,
			kind: t,
			tokenIndex: n,
			scopeId: r,
			functionId: i,
			safeToRename: a
		}), o === void 0 ? this.activateSymbol(e, s, r) : this.pendingSymbols.push({
			name: e,
			id: s,
			scopeId: r,
			activateAfter: o
		}), s;
	}
	activatePendingSymbols(e) {
		for (let t = this.pendingSymbols.length - 1; t >= 0; t--) {
			let n = this.pendingSymbols[t];
			n.activateAfter >= e || (this.activateSymbol(n.name, n.id, n.scopeId), this.pendingSymbols.splice(t, 1));
		}
	}
	activateSymbol(e, t, n) {
		let r = this.symbolsByScope.get(n);
		r || (r = /* @__PURE__ */ new Map(), this.symbolsByScope.set(n, r)), r.has(e) || r.set(e, t);
	}
	resolve(e, t) {
		for (let n = t.length - 1; n >= 0; n--) {
			let r = this.symbolsByScope.get(t[n])?.get(e);
			if (r !== void 0) return r;
		}
	}
	preserveAttribute(e) {
		this.preserveToken(e, "attribute");
		let t = this.nextSig(e);
		if (t === void 0) return e;
		this.preserveToken(t, "attribute");
		let n = this.nextSig(t);
		if (n === void 0 || this.tokens[n]?.text !== "(") return t;
		let r = this.findMatching(n, "(", ")");
		return r === void 0 ? (this.preserveRange(n, n, "attribute"), n) : (this.preserveRange(n, r, "attribute"), r);
	}
	preserveTypeFrom(e, t, n) {
		let r = 0, i = 0, a = 0, o = e - 1;
		for (let s = e; s < n; s++) {
			let n = this.tokens[s];
			if (!G(n)) {
				if (r === 0 && i === 0 && a === 0 && t.includes(n.text)) return Math.max(e - 1, s - 1);
				if (n.text === "<") r++;
				else if (n.text === ">") r = Math.max(0, r - 1);
				else if (n.text === "(") i++;
				else if (n.text === ")") {
					if (i === 0 && t.includes(")")) return Math.max(e - 1, s - 1);
					i = Math.max(0, i - 1);
				} else n.text === "[" ? a++ : n.text === "]" && (a = Math.max(0, a - 1));
				n.kind === "ident" && this.preserveToken(s, "type"), o = s;
			}
		}
		return o;
	}
	preserveStatement(e, t) {
		let n = this.findStatementEnd(e);
		return this.preserveRange(e, n, t), n;
	}
	preserveRange(e, t, n) {
		for (let r = e; r <= t; r++) this.tokens[r] && this.tokens[r].kind !== "lineComment" && this.tokens[r].kind !== "blockComment" && this.preserveToken(r, n);
	}
	preserveToken(e, t) {
		this.preserved.has(e) || this.preserved.set(e, t);
	}
	createScope(e, t, n, r) {
		let i = this.scopes.length;
		return this.scopes.push({
			id: i,
			kind: e,
			parentId: t,
			functionId: n,
			startToken: r
		}), i;
	}
	nextSig(e) {
		for (let t = e + 1; t < this.tokens.length; t++) if (!G(this.tokens[t])) return t;
	}
	findNextIdent(e) {
		for (let t = e; t < this.tokens.length; t++) {
			let e = this.tokens[t];
			if (!G(e)) {
				if (e.kind === "ident") return t;
				if (e.text !== "@") return;
			}
		}
	}
	findNextText(e, t) {
		for (let n = e; n < this.tokens.length; n++) if (!G(this.tokens[n]) && this.tokens[n].text === t) return n;
	}
	findStatementEnd(e) {
		let t = 0;
		for (let n = e; n < this.tokens.length; n++) {
			let e = this.tokens[n].text;
			if (e === "(") t++;
			else if (e === ")") t = Math.max(0, t - 1);
			else if (t === 0 && (e === ";" || e === "{" || e === "}")) return n;
		}
		return this.tokens.length - 1;
	}
	findMatching(e, t, n) {
		let r = 0;
		for (let i = e; i < this.tokens.length; i++) {
			let e = this.tokens[i].text;
			if (e === t && r++, e === n && (r--, r === 0)) return i;
		}
	}
	hasEntryAttributeBefore(e) {
		for (let t = e - 1; t >= 0; t--) {
			let e = this.tokens[t];
			if (!G(e)) {
				if (e.text === ")" || e.kind === "ident" || e.text === "@") {
					let t = e.text;
					if (t === "compute" || t === "vertex" || t === "fragment") return !0;
					continue;
				}
				break;
			}
		}
		return !1;
	}
	moduleFallback(e, t) {
		this.moduleFallbackReasons.push(`${e} at token ${t}`);
	}
	functionFallback(e, t, n) {
		e.skipped = !0, e.fallbackReasons.push(`${t} at token ${n}`);
	}
};
function $r(e, t) {
	for (let n = e.length - 1; n >= 0; n--) if (t(e[n])) return e[n];
}
function G(e) {
	return e.kind === "lineComment" || e.kind === "blockComment";
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-sampling.js
var ei = /* @__PURE__ */ new Set([
	"textureSample",
	"textureSampleBias",
	"textureSampleLevel",
	"textureSampleGrad",
	"textureGather",
	"textureSampleBaseClampToEdge"
]), ti = /* @__PURE__ */ new Set([
	"textureSampleCompare",
	"textureSampleCompareLevel",
	"textureGatherCompare"
]);
function ni(e, t, n) {
	let r = /* @__PURE__ */ new Map();
	for (let i = 0; i < e.length; i++) {
		let a = e[i], o = t[i], s = Zr(a.tokens), c = /* @__PURE__ */ new Map();
		for (let e of o.vars) {
			let t = z(e.attrs, "group"), n = z(e.attrs, "binding"), r = s.declarations.find((t) => t.kind === "global" && t.name === e.name);
			t !== void 0 && n !== void 0 && r && c.set(r.id, {
				group: t,
				binding: n
			});
		}
		let l = /* @__PURE__ */ new Map();
		for (let e of s.declarations) {
			if (e.kind !== "function") continue;
			let t = s.functions.find((t) => t.nameTokenIndex === e.tokenIndex);
			t && l.set(e.id, t.id);
		}
		for (let e of o.entries) {
			let t = s.functions.find((t) => t.name === e.name), i = [], a = s.fallback.wholeModule || !t;
			!a && t && (a = !ri(t.id, /* @__PURE__ */ new Map(), /* @__PURE__ */ new Set(), s, c, l, i));
			let o = t ? ci(t.id, s, c, l) : n.map(ui);
			r.set(e, a ? li(n, o) : di(i));
		}
	}
	return r;
}
function ri(e, t, n, r, i, a, o) {
	let s = r.functions[e];
	if (!s || s.skipped) return !1;
	let c = `${e}|${[...t].map(([e, t]) => `${e}:${t.group}:${t.binding}`).join(",")}`;
	if (n.has(c)) return !0;
	n.add(c);
	let l = r.references.filter((t) => t.functionId === e), u = new Map(l.map((e) => [e.tokenIndex, e]));
	for (let e = s.bodyStartToken + 1; e < s.bodyEndToken; e++) {
		let s = r.tokens[e]?.text, c = ei.has(s ?? "") ? "filtering" : ti.has(s ?? "") ? "comparison" : void 0, l = u.get(e), d = l && a.get(l.declarationId);
		if (!c && d === void 0) continue;
		let f = si(r, e);
		if (f === void 0 || r.tokens[f]?.text !== "(") continue;
		let p = oi(r, f);
		if (!p) return !1;
		let m = p.map(([e, n]) => ii(e, n, r, i, t));
		if (c) {
			let e = +(s === "textureGather" && !ai(p[0], r, i, t)), n = m[e], a = m[e + 1];
			if (!n || !a) return !1;
			o.push({
				texture: n,
				sampler: a,
				mode: c
			});
		} else {
			let e = r.declarations.filter((e) => e.kind === "param" && e.functionId === d).sort((e, t) => e.tokenIndex - t.tokenIndex), t = /* @__PURE__ */ new Map();
			for (let n = 0; n < e.length; n++) m[n] && t.set(e[n].id, m[n]);
			if (!ri(d, t, n, r, i, a, o)) return !1;
		}
	}
	return !0;
}
function ii(e, t, n, r, i) {
	for (let a of n.references) {
		if (a.tokenIndex < e || a.tokenIndex > t) continue;
		let n = r.get(a.declarationId) ?? i.get(a.declarationId);
		if (n) return n;
	}
}
function ai(e, t, n, r) {
	let i = t.references.find((t) => t.tokenIndex >= e[0] && t.tokenIndex <= e[1]);
	return i?.tokenIndex === e[0] ? n.get(i.declarationId) ?? r.get(i.declarationId) : void 0;
}
function oi(e, t) {
	let n = [], r = 1, i = 0, a = 0, o = 0, s = t + 1;
	for (let c = t + 1; c < e.tokens.length; c++) {
		let t = e.tokens[c].text;
		if (t === "(") r++;
		else if (t === ")") {
			if (r--, r === 0) return n.push([s, c - 1]), n;
		} else t === "[" ? i++ : t === "]" ? i-- : t === "{" ? a++ : t === "}" ? a-- : t === "<" ? o++ : t === ">" ? o-- : t === "," && r === 1 && i === 0 && a === 0 && o === 0 && (n.push([s, c - 1]), s = c + 1);
	}
}
function si(e, t) {
	for (let n = t + 1; n < e.tokens.length; n++) if (e.tokens[n].kind !== "lineComment" && e.tokens[n].kind !== "blockComment") return n;
}
function ci(e, t, n, r) {
	let i = [e], a = /* @__PURE__ */ new Set(), o = /* @__PURE__ */ new Map();
	for (; i.length;) {
		let e = i.pop();
		if (!a.has(e)) {
			a.add(e);
			for (let a of t.references) {
				if (a.functionId !== e) continue;
				let t = n.get(a.declarationId);
				t && o.set(`${t.group}:${t.binding}`, t);
				let s = r.get(a.declarationId);
				s !== void 0 && i.push(s);
			}
		}
	}
	return [...o.values()];
}
function li(e, t) {
	let n = new Set(t.map((e) => `${e.group}:${e.binding}`)), r = e.filter((e) => n.has(`${e.group}:${e.binding}`)), i = r.filter((e) => e.bindingLayout?.kind === "texture" && e.bindingLayout.texture.sampleType === "unfilterable-float" && !e.bindingLayout.texture.multisampled), a = r.filter((e) => e.bindingLayout?.kind === "sampler" && e.bindingLayout.sampler.type === "filtering");
	return i.flatMap((e) => a.map((t) => ({
		texture: ui(e),
		sampler: ui(t),
		mode: "filtering"
	})));
}
function ui(e) {
	return {
		group: e.group,
		binding: e.binding
	};
}
function di(e) {
	let t = /* @__PURE__ */ new Set();
	return e.filter((e) => {
		let n = `${e.texture.group}:${e.texture.binding}:${e.sampler.group}:${e.sampler.binding}:${e.mode}`;
		return !t.has(n) && (t.add(n), !0);
	});
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect.js
function fi(e, t) {
	let n = e.map(Bn), r = ar(e, n, t), i = sr(n, r), a = [], o = [];
	for (let e of n) for (let t of e.vars) {
		let e = z(t.attrs, "group"), n = z(t.attrs, "binding");
		if (e === void 0 || n === void 0) continue;
		let s = W(t.type, t.path, r, i), c = en(s, t.addressSpace), l = t.addressSpace === "uniform" || t.addressSpace === "storage" ? pr(s, t.addressSpace, t.name, t.mangledName, i) : void 0;
		l && o.push(l), a.push({
			group: e,
			binding: n,
			name: t.name,
			mangledName: t.mangledName,
			type: s,
			kind: c,
			addressSpace: t.addressSpace,
			access: t.access,
			struct: s.kind === "identifier" ? i.structs.get(s.mangledName ?? s.name) : void 0,
			layout: l,
			bindingLayout: tn(c, t.addressSpace, t.access, s, l)
		});
	}
	a.sort((e, t) => e.group - t.group || e.binding - t.binding);
	let s = pi(e, n, a), c = ni(e, n, a);
	return {
		bindings: a,
		entryPoints: n.flatMap((e) => e.entries.map((e) => mi(e, n.flatMap((e) => e.structs), r, i, s.get(e) ?? a, c.get(e) ?? []))),
		overrides: n.flatMap((e) => e.overrides),
		featuresRequired: [...new Set(n.flatMap((e) => e.features))],
		aliases: [...i.aliases.values()],
		structs: [...i.structs.values()],
		hostShareableLayouts: o
	};
}
function pi(e, t, n) {
	let r = /* @__PURE__ */ new Map();
	for (let i = 0; i < e.length; i++) {
		let a = e[i], o = t[i], s = Zr(a.tokens), c = s.fallback.wholeModule, l = /* @__PURE__ */ new Map();
		for (let e of s.declarations) {
			if (e.kind !== "function") continue;
			let t = s.functions.find((t) => t.nameTokenIndex === e.tokenIndex);
			t && l.set(e.id, t.id);
		}
		let u = /* @__PURE__ */ new Map();
		for (let e of o.vars) {
			let t = z(e.attrs, "group"), n = z(e.attrs, "binding");
			if (t === void 0 || n === void 0) continue;
			let r = s.declarations.find((t) => t.kind === "global" && t.name === e.name);
			r && u.set(r.id, {
				group: t,
				binding: n
			});
		}
		for (let e of o.entries) {
			let t = s.functions.find((t) => t.name === e.name);
			if (c || !t) {
				r.set(e, n);
				continue;
			}
			let i = [t.id], a = /* @__PURE__ */ new Set(), o = /* @__PURE__ */ new Map();
			for (; i.length;) {
				let e = i.pop();
				if (!a.has(e) && (a.add(e), s.functions[e])) for (let t of s.references) {
					if (t.functionId !== e) continue;
					let n = u.get(t.declarationId);
					n && o.set(`${n.group}:${n.binding}`, n);
					let r = l.get(t.declarationId);
					r !== void 0 && i.push(r);
				}
			}
			r.set(e, [...o.values()].sort((e, t) => e.group - t.group || e.binding - t.binding));
		}
	}
	return r;
}
function mi(e, t, n, r, i, a) {
	return {
		name: e.name,
		mangledName: e.mangledName,
		stage: e.stage,
		...e.workgroupSize ? { workgroupSize: e.workgroupSize } : {},
		bindings: i.map(({ group: e, binding: t }) => ({
			group: e,
			binding: t
		})),
		samplingPairs: a,
		...e.stage === "vertex" ? { inputs: hi(e, t, n, r) } : {}
	};
}
function hi(e, t, n, r) {
	let i = [];
	for (let a of e.params) {
		if (gi(a.attrs, "builtin")) continue;
		let o = W(a.type, e.path, n, r), s = z(a.attrs, "location");
		if (s !== void 0) {
			i.push({
				name: a.name,
				location: s,
				type: o
			});
			continue;
		}
		let c = cr(o, r);
		if (c.kind !== "identifier") continue;
		let l = t.find((e) => e.mangledName === (c.mangledName ?? c.name)), u = r.structs.get(c.mangledName ?? c.name);
		if (l) for (let e = 0; e < l.members.length; e++) {
			let t = l.members[e];
			if (gi(t.attrs, "builtin")) continue;
			let a = z(t.attrs, "location");
			a !== void 0 && i.push({
				name: t.name,
				location: a,
				type: u?.members[e]?.type ?? W(t.type, l.path, n, r)
			});
		}
	}
	return i;
}
function gi(e, t) {
	return e.some((e) => e.name === t);
}
//#endregion
//#region node_modules/@vgpu/wgsl/dist/runtime/reflect-source.js
function _i(e, t = "<runtime>") {
	let n = Gr(e, t), r = qt(n);
	if (r.imports.length > 0) throw P("VGPU-WGSL-REFLECT-SOURCE-IMPORT", "reflectSource() accepts a single raw WGSL string; use resolveShader() for WGSL import graphs.");
	return fi([{
		path: t,
		source: e,
		tokens: n,
		parsed: r
	}]);
}
//#endregion
//#region node_modules/vgpu/dist/bind-cache.js
function vi() {
	let e = /* @__PURE__ */ new Map();
	return {
		getOrCreate(t, n, r, i) {
			let a = r.map(yi), o = `${t}:${n}:${a.join("|")}`, s = e.get(o);
			if (s) return s.bindGroup;
			let c = i();
			return e.set(o, {
				identities: a,
				bindGroup: c
			}), c;
		},
		evictIdentity(t) {
			let n = yi(t);
			for (let [t, r] of e) r.identities.includes(n) && e.delete(t);
		},
		clearDraw(t) {
			let n = `${t}:`;
			for (let t of e.keys()) t.startsWith(n) && e.delete(t);
		},
		dispose() {
			e.clear();
		}
	};
}
function yi(e) {
	return typeof e == "string" || typeof e == "number" ? String(e) : `${e.kind}:${e.id}`;
}
//#endregion
//#region node_modules/vgpu/dist/entry-metadata.js
function bi(e, t, n) {
	let r = e[t];
	if (!r) throw new E({
		code: "VGPU-REFLECT-ENTRY-METADATA-MISSING",
		message: `Entry point '${e.name}' has no reflected ${t}.`,
		fix: "Pass the reflection from reflectSource()/resolveShader().",
		where: n
	});
	return r;
}
//#endregion
//#region node_modules/vgpu/dist/claim-validation.js
var xi = /* @__PURE__ */ new WeakMap();
function Si(e, t) {
	if (!e.gpu.pushErrorScope || !e.gpu.popErrorScope) return;
	e.gpu.pushErrorScope("validation");
	let n = xi.get(e.gpu);
	n ? n.push(t) : xi.set(e.gpu, [t]);
}
function K(e) {
	let t = xi.get(e.gpu);
	if (!t?.length || !e.gpu.popErrorScope) return;
	let n = t.pop();
	return t.length || xi.delete(e.gpu), {
		context: n,
		error: e.gpu.popErrorScope()
	};
}
function Ci(e) {
	let t = [], n = K(e);
	for (; n;) t.push(n), n = K(e);
	return t;
}
function wi(e) {
	let t = K(e);
	t && ji(t);
}
function Ti(e) {
	for (let t of Ci(e)) ji(t);
}
function q(e) {
	for (let t of e) ji(t);
}
function Ei(e) {
	return e.gpu.queue.onSubmittedWorkDone?.() ?? Promise.resolve();
}
function Di(e, t = [], n = {}) {
	return Ai(e, t, n.errorSink ?? Mi);
}
function Oi(e, t) {
	return {
		context: e.context,
		error: ki(e.error, t.error)
	};
}
async function ki(e, t) {
	let n = await Promise.allSettled([e, t]);
	for (let e of n) if (e.status === "fulfilled" && e.value) return e.value;
	let r = n.find((e) => e.status === "rejected");
	if (r?.status === "rejected") throw r.reason;
	return null;
}
async function Ai(e, t, n) {
	await Ei(e);
	for (let e of t) try {
		let t = await e.error;
		t && await n(D(e.context.label, e.context.group, t));
	} catch (t) {
		await n(D(e.context.label, e.context.group, t));
	}
}
function ji(e) {
	e.error.catch(() => void 0);
}
function Mi(e) {
	console.error(e);
}
//#endregion
//#region node_modules/vgpu/dist/claim-validation-encode.js
function Ni(e, t, n, r) {
	try {
		t.end();
	} catch (t) {
		let i = Ci(e);
		q(n), q(i), n.length = 0;
		let a = i[0]?.context ?? r;
		throw a ? D(a.label, a.group, t) : t;
	}
}
//#endregion
//#region node_modules/vgpu/dist/set-resources.js
var Pi = 1, Fi = /* @__PURE__ */ new WeakMap();
function Ii(e) {
	return typeof e != "object" || !e || ArrayBuffer.isView(e) || e instanceof ArrayBuffer || Array.isArray(e) ? !0 : e instanceof g || e instanceof pe ? !1 : !qi(e);
}
function Li(e) {
	return typeof e != "object" || !e || Array.isArray(e) || ArrayBuffer.isView(e) || e instanceof ArrayBuffer || e instanceof g || e instanceof pe ? !1 : !qi(e);
}
function Ri(e, t, n) {
	switch (e.bindingLayout?.kind) {
		case "buffer": return zi(e, t, n);
		case "texture": return Bi(e, t, n);
		case "sampler": return Vi(e, t);
		case "storageTexture": throw j(e, "storage texture", "Pass a storage-compatible texture.");
		case "externalTexture": throw j(e, "external texture", "Pass a compatible GPUExternalTexture.");
		default: throw j(e, "reflected resource", "Fix shader reflection bindingLayout.");
	}
}
function zi(e, t, n) {
	let r = zt(t);
	if (r) return r[Rt](e, n.sourceHint);
	if (t instanceof g) return Lt(t, `${n.sourceHint}.set`), Ui(e, t.options.usage), {
		resource: { buffer: t.gpu },
		identity: t.resourceIdentity,
		unsubscribe: (e) => t.onDestroy(e)
	};
	if (Yi(t)) return Lt(t.buffer, `${n.sourceHint}.set`), {
		resource: {
			buffer: t.gpu,
			offset: 0,
			size: t.size
		},
		identity: t.buffer.resourceIdentity,
		unsubscribe: (e) => t.buffer.onDestroy(e)
	};
	if (Zi(t)) return {
		resource: t,
		identity: Ji(t.buffer)
	};
	if (Qi(t)) return {
		resource: { buffer: t },
		identity: Ji(t)
	};
	throw j(e, "buffer", `Pass a compatible Buffer/Uniform: ${e.name}.set({ ${e.name}: gpu.device.createBuffer(...) }).`);
}
function Bi(e, t, n) {
	let r = Ki(t);
	if (r) {
		let t = r.color;
		Gi(e, t, n);
		let i = r.onTexturesRecreated?.bind(r);
		return {
			resource: t.createView(),
			identity: t.resourceIdentity,
			unsubscribe: (e) => r.onDestroy(e),
			onRecreate: i ? (e) => i(e) : void 0
		};
	}
	if (t instanceof pe) return Wi(e, t.usage), Gi(e, t, n), {
		resource: t.createView(),
		identity: t.resourceIdentity,
		unsubscribe: (e) => t.onDestroy(e)
	};
	if (Xi(t)) return {
		resource: t.createView(),
		identity: t.resourceIdentity ?? Ji(t)
	};
	if (typeof t == "object" && t) return {
		resource: t,
		identity: Ji(t)
	};
	throw j(e, "texture/target", `Pass a Texture or Target: ${e.name}.set({ ${e.name}: scene.color }) or set({ ${e.name}: scene }).`);
}
function Vi(e, t) {
	if (Hi(t)) return {
		resource: t,
		identity: Ji(t)
	};
	throw j(e, "sampler", `Use the cached sampler: set({ ${e.name}: sampler(gpu) }).`);
}
function Hi(e) {
	return typeof e != "object" || !e || e instanceof g || e instanceof pe ? !1 : !Qi(e) && !Zi(e) && !Xi(e) && !Ki(e);
}
function Ui(e, t) {
	let n = e.bindingLayout?.kind === "buffer" ? e.bindingLayout.buffer.type : void 0;
	if (n === "uniform" && !t.includes("uniform")) throw j(e, "uniform buffer", "Create with usage: ['uniform','copy_dst'].");
	if ((n === "storage" || n === "read-only-storage") && !t.includes("storage")) throw j(e, "storage buffer", "Create with usage: ['storage','copy_dst'].");
}
function Wi(e, t) {
	if (!t.includes("texture_binding") && !t.includes("render_attachment")) throw j(e, "sampled texture", "Use texture_binding usage or a sampleable Target.");
}
function Gi(e, t, n) {
	if (!(!n.filterableTexture || n.float32Filterable) && (t.format === "r32float" || t.format === "rg32float" || t.format === "rgba32float")) throw Ae(n.sourceHint, e, t.format, t.label ?? "texture", n.pairedSampler);
}
function Ki(e) {
	if (typeof e != "object" || !e) return;
	let t = e;
	if (!(!t.resourceIdentity || !t.color || typeof t.onDestroy != "function")) return t;
}
function qi(e) {
	let t = e;
	return "gpu" in t || "bindGroup" in t || "createView" in t || "resourceIdentity" in t;
}
function Ji(e) {
	if (typeof e != "object" || !e) return `value:${String(e)}`;
	let t = Fi.get(e);
	return t || (t = {
		kind: "external",
		id: Pi++
	}, Fi.set(e, t)), t;
}
function Yi(e) {
	return typeof e == "object" && !!e && "gpu" in e && "size" in e && "buffer" in e && e.buffer instanceof g;
}
function Xi(e) {
	return typeof e == "object" && !!e && typeof e.createView == "function";
}
function Zi(e) {
	return typeof e == "object" && !!e && "buffer" in e && Qi(e.buffer);
}
function Qi(e) {
	return typeof e == "object" && !!e && "size" in e && "usage" in e && typeof e.destroy == "function";
}
//#endregion
//#region node_modules/vgpu/dist/set-packing.js
function $i(e, t) {
	ea(e);
	let n = new ArrayBuffer(e.size);
	return ta(new DataView(n), e, 0, t), n;
}
function ea(e) {
	if (e.size === void 0) throw M("set", `No se puede inferir byteLength para layout runtime-sized '${e.name}'.`);
}
function ta(e, t, n, r) {
	if (t.members) return na(e, t.members, n, r);
	ra(e, t, n, r);
}
function na(e, t, n, r) {
	let i = r;
	for (let r of t) ta(e, r.layout, n + r.offset, i?.[r.name]);
}
function ra(e, t, n, r) {
	switch (t.type.kind) {
		case "scalar": return ia(e, n, t.type.name, r);
		case "vector": return aa(e, n, t.type, r);
		case "matrix": return oa(e, t, n, r);
		case "array": return sa(e, t, n, r);
		default: throw M("set", `No hay writer para layout ${t.type.kind}.`);
	}
}
function ia(e, t, n, r) {
	n === "f32" ? e.setFloat32(t, Number(r ?? 0), !0) : n === "i32" ? e.setInt32(t, Number(r ?? 0), !0) : n === "u32" || n === "bool" ? e.setUint32(t, n === "bool" ? +!!r : Number(r ?? 0), !0) : e.setUint16(t, ua(Number(r ?? 0)), !0);
}
function aa(e, t, n, r) {
	let i = r, a = ca(n.element);
	for (let r = 0; r < n.width; r++) ia(e, t + r * a, la(n.element), i?.[r] ?? 0);
}
function oa(e, t, n, r) {
	let i = t.type, a = r, o = ca(i.element), s = t.stride ?? 16;
	for (let t = 0; t < i.columns; t++) for (let r = 0; r < i.rows; r++) ia(e, n + t * s + r * o, la(i.element), a?.[t * i.rows + r] ?? 0);
}
function sa(e, t, n, r) {
	let i = r, a = t.stride ?? t.element?.size ?? 0;
	if (!t.element) throw M("set", "Array layout sin element layout.");
	for (let r = 0; r < (i?.length ?? 0); r++) ta(e, t.element, n + r * a, i[r]);
}
function ca(e) {
	return la(e) === "f16" ? 2 : 4;
}
function la(e) {
	if (e.kind !== "scalar") throw M("set", `Expected scalar, got ${e.kind}`);
	return e.name;
}
function ua(e) {
	let t = /* @__PURE__ */ new Float32Array(1), n = new Uint32Array(t.buffer);
	t[0] = e;
	let r = n[0], i = r >> 16 & 32768, a = r & 8388607, o = r >> 23 & 255;
	if (o === 255) return i | (a ? 32256 : 31744);
	let s = o - 127 + 15;
	return s >= 31 ? i | 31744 : s <= 0 ? s < -10 ? i : i | (a | 8388608) >> 1 - s + 13 : i | s << 10 | a >> 13;
}
//#endregion
//#region node_modules/vgpu/dist/set-layouts.js
var da = /* @__PURE__ */ new WeakMap();
function fa(e, t) {
	let n = /* @__PURE__ */ new Map(), r = /* @__PURE__ */ new Set();
	for (let e of t) {
		let t = e.stage === "vertex" ? 1 : e.stage === "fragment" ? 2 : 4;
		for (let r of bi(e, "bindings", "visibility")) {
			let e = `${r.group}:${r.binding}`;
			n.set(e, (n.get(e) ?? 0) | t);
		}
		for (let t of bi(e, "samplingPairs", "visibility")) t.mode === "filtering" && r.add(`${t.texture.group}:${t.texture.binding}`);
	}
	let i = (e) => n.get(`${e.group}:${e.binding}`) ?? 0;
	return Object.defineProperty(i, "filterable", { value: r }), i;
}
function pa(e, t, n = ya) {
	return e.flatMap((e) => {
		if (e.group !== t) return [];
		let r = n(e);
		return r === 0 ? [] : [{
			binding: e.binding,
			visibility: r,
			..._a(e, n.filterable?.has(`${e.group}:${e.binding}`) ?? !1)
		}];
	});
}
function ma(e, t, n, r = ya) {
	let i = /* @__PURE__ */ new Map(), a = n.bindings.filter((e) => r(e) !== 0).map((e) => e.group), o = Math.max(-1, ...a);
	for (let a = 0; a <= o; a++) i.set(a, ha(e, t, n, a, r));
	return i;
}
function ha(e, t, n, r, i = ya) {
	return ga(e, `${t}.group${r}.bgl`, pa(n.bindings, r, i));
}
function ga(e, t, n) {
	let r = da.get(e.gpu);
	r || (r = /* @__PURE__ */ new Map(), da.set(e.gpu, r));
	let i = JSON.stringify(n), a = r.get(i);
	if (a) return a;
	let o = Te(e.gpu.createBindGroupLayout({
		label: t,
		entries: n
	}), { entries: n });
	return r.set(i, o), o;
}
function _a(e, t) {
	let n = e.bindingLayout;
	if (!n) throw M("bindGroupLayout", `Binding '${e.name}' does not have a reflected bindingLayout.`);
	return t && n.kind === "texture" && n.texture.sampleType === "unfilterable-float" && !n.texture.multisampled ? { texture: {
		...n.texture,
		sampleType: "float"
	} } : va(n);
}
function va(e) {
	switch (e.kind) {
		case "buffer": return { buffer: { ...e.buffer } };
		case "sampler": return { sampler: { ...e.sampler } };
		case "texture": return { texture: { ...e.texture } };
		case "storageTexture": return { storageTexture: { ...e.storageTexture } };
		case "externalTexture": return { externalTexture: {} };
	}
}
function ya(e) {
	let t = globalThis.GPUShaderStage, n = t?.VERTEX ?? 1, r = t?.FRAGMENT ?? 2, i = t?.COMPUTE ?? 4;
	return e.kind === "buffer" ? n | r | i : r | i;
}
//#endregion
//#region node_modules/vgpu/dist/set-core.js
function ba(e) {
	let t = xa(e.reflection), n = [...e.bindGroupLayouts.keys()].sort((e, t) => e - t), r = /* @__PURE__ */ new Map();
	function i(e) {
		let t = [];
		for (let [n, r] of Object.entries(e)) t.push(...o(n, r));
		return t;
	}
	function a(t) {
		let n = e.bindGroupLayouts.get(t.info.group);
		return !!n && !!Ee(n)?.entries.some((e) => e.binding === t.info.binding);
	}
	function o(n, r) {
		let i = t.get(n);
		if (i) return s(i, n, r);
		let a = Sa(n, t, e.label);
		if (!a) throw M(`${e.label}.set`, `Binding '${n}' does not exist in '${e.label}'.`);
		return c(a, n, r);
	}
	function s(e, t, n) {
		x(e.info.group);
		let r = Ca(e.info, n);
		wa(e, t, r);
		let i = ja(e.identity);
		return r === "lib" ? l(e, Ma(e.libValue, n)) : d(e, n), a(e) ? Aa(e, i) : [];
	}
	function c(t, n, r) {
		x(t.info.group);
		let i = Ca(t.info, r);
		if (wa(t, n, i), Ta(t, n, i), i !== "lib") throw M(`${e.label}.set`, `Member '${n}' needs a JS value; set resource '${t.info.name}' instead.`);
		let o = ja(t.identity);
		return l(t, {
			...Na(t.libValue),
			[n]: r
		}), a(t) ? Aa(t, o) : [];
	}
	function l(e, t) {
		let n = C(e);
		e.libValue = t;
		let r = $i(n, t);
		e.buffer || S(e, n.size), e.bytes = r, e.buffer.write(r, 0);
	}
	function u(t) {
		let n = Ee(e.bindGroupLayouts.get(t.group))?.entries.find((e) => e.binding === t.binding), r = e.reflection.entryPoints.flatMap((t) => bi(t, "samplingPairs", e.label)).find((e) => e.mode === "filtering" && e.texture.group === t.group && e.texture.binding === t.binding), i = r && e.reflection.bindings.find((e) => e.group === r.sampler.group && e.binding === r.sampler.binding);
		return {
			sourceHint: e.label,
			filterableTexture: n?.texture?.sampleType === "float",
			float32Filterable: e.device.features.has("float32-filterable"),
			pairedSampler: i
		};
	}
	function d(t, n) {
		let r = Ri(t.info, n, u(t.info));
		t.unsubscribe?.(), t.unsubscribeRecreate?.(), t.resource = r.resource, t.identity = r.identity, t.unsubscribe = r.unsubscribe?.(() => {
			t.identity && e.cache.evictIdentity(t.identity);
		}), t.unsubscribeRecreate = r.onRecreate?.(() => f(t, n));
	}
	function f(t, n) {
		let r = ja(t.identity);
		t.identity && e.cache.evictIdentity(t.identity);
		let i = Ri(t.info, n, u(t.info));
		if (t.unsubscribe?.(), t.unsubscribeRecreate?.(), t.resource = i.resource, t.identity = i.identity, t.unsubscribe = i.unsubscribe?.(() => {
			t.identity && e.cache.evictIdentity(t.identity);
		}), t.unsubscribeRecreate = i.onRecreate?.(() => f(t, n)), a(t)) for (let n of Aa(t, r)) e.onIdentityChange?.(n);
	}
	function p(t, n, i) {
		m(t), Ea(e.label, t, n, i);
		let a = r.has(t) ? `claimed-group:${t}` : void 0;
		return r.set(t, n), a;
	}
	function m(t) {
		let n = e.bindGroupLayouts.get(t);
		if (!n) throw M(`${e.label}.layout`, `@group(${t}) does not exist in '${e.label}'.`);
		return n;
	}
	function h() {
		return n.map(g);
	}
	function g(t) {
		let n = r.get(t);
		if (n) return {
			group: t,
			bindGroup: n,
			offsets: [],
			claimValidation: _(n, t)
		};
		let i = new Set(Ee(m(t))?.entries.map((e) => e.binding)), a = e.reflection.bindings.filter((e) => e.group === t && i.has(e.binding)), o = v(a), s = y(a);
		return {
			group: t,
			bindGroup: e.cache.getOrCreate(e.drawId, t, s, () => e.device.gpu.createBindGroup({
				label: `${e.label}.group${t}`,
				layout: m(t),
				entries: o
			})),
			offsets: []
		};
	}
	function _(t, n) {
		return De(t) ? void 0 : {
			label: e.label,
			group: n
		};
	}
	function v(e) {
		return e.map((e) => {
			let t = b(e);
			return {
				binding: e.binding,
				resource: t.resource
			};
		});
	}
	function y(e) {
		return e.map((e) => b(e).identity);
	}
	function b(n) {
		let r = t.get(n.name);
		if (!r?.resource || !r.identity) throw je(e.label, n);
		return r;
	}
	function x(t) {
		if (r.has(t)) throw Ne(e.label, t);
	}
	function S(t, n) {
		t.buffer = e.device.createBuffer({
			size: n,
			usage: ["uniform", "copy_dst"],
			label: `${e.label}.${t.info.name}`
		}), t.resource = {
			buffer: t.buffer.gpu,
			offset: 0,
			size: n
		}, t.identity = t.buffer.resourceIdentity, t.unsubscribe = t.buffer.onDestroy(() => e.cache.evictIdentity(t.buffer.resourceIdentity));
	}
	function C(t) {
		if (t.info.kind !== "buffer" || !t.info.layout?.size) throw M(`${e.label}.set`, `Binding '${t.info.name}' needs a compatible resource, not JS.`);
		return t.info.layout;
	}
	return {
		get groups() {
			return n;
		},
		set: i,
		claimGroup: p,
		layout: m,
		bindGroups: h,
		bindingState(e) {
			let n = t.get(e);
			if (!(!n?.ownership || !n.resource || !n.identity)) return {
				info: n.info,
				ownership: n.ownership,
				resource: n.resource,
				identity: n.identity
			};
		}
	};
}
function xa(e) {
	return new Map(e.bindings.map((e) => [e.name, {
		info: e,
		memberOwnership: /* @__PURE__ */ new Map()
	}]));
}
function Sa(e, t, n) {
	let r;
	for (let i of t.values()) if (i.info.layout?.members?.some((t) => t.name === e)) {
		if (r) throw M(`${n}.set`, `Binding member '${e}' is ambiguous in '${n}'; set the complete binding.`);
		r = i;
	}
	return r;
}
function Ca(e, t) {
	return e.bindingLayout?.kind === "buffer" && Ii(t) ? "lib" : "user";
}
function wa(e, t, n) {
	if (e.ownership && e.ownership !== n) throw Me(t, e.ownership);
	e.ownership ??= n;
}
function Ta(e, t, n) {
	let r = e.memberOwnership.get(t);
	if (r && r !== n) throw Me(t, r);
	e.memberOwnership.set(t, n);
}
function Ea(e, t, n, r) {
	let i = De(n);
	if (!i) return;
	let a = Ee(r);
	if (!a) return;
	let o = Da(a.entries, i.layout.entries);
	if (o) throw Pe(e, t, o);
}
function Da(e, t) {
	if (e.length !== t.length) return `expected ${e.length} bindings and received ${t.length}`;
	let n = Oa(e), r = Oa(t);
	for (let [e, t] of n) {
		let n = r.get(e);
		if (!n) return `missing @binding(${e})`;
		if (ka(t) !== ka(n)) return `@binding(${e}) does not match the reflected layout`;
	}
}
function Oa(e) {
	return new Map(e.map((e) => [e.binding, e]));
}
function ka(e) {
	return JSON.stringify({
		binding: e.binding,
		visibility: e.visibility,
		buffer: e.buffer,
		sampler: e.sampler,
		texture: e.texture,
		storageTexture: e.storageTexture,
		externalTexture: e.externalTexture ? {} : void 0
	});
}
function Aa(e, t) {
	let n = ja(e.identity);
	return !n || t === n ? [] : [{
		group: e.info.group,
		binding: e.info.binding,
		bindingName: e.info.name,
		bindingKind: e.info.kind,
		previousIdentity: t,
		newIdentity: n
	}];
}
function ja(e) {
	return e === void 0 ? void 0 : yi(e);
}
function Ma(e, t) {
	return Li(e) && Li(t) ? {
		...e,
		...t
	} : t;
}
function Na(e) {
	return Li(e) ? e : {};
}
var Pa = Object.freeze([
	0,
	0,
	0,
	1
]);
function Fa(e, t) {
	let n = e, r = Array.isArray(e) ? e : [
		n?.r,
		n?.g,
		n?.b,
		n?.a
	];
	if (r.length !== 4 || !r.every((e) => typeof e == "number" && Number.isFinite(e))) throw yt(t);
	return Ia(e);
}
function Ia(e) {
	let t = e;
	return Array.isArray(e) ? [
		e[0],
		e[1],
		e[2],
		e[3]
	] : {
		r: t.r,
		g: t.g,
		b: t.b,
		a: t.a
	};
}
function La(e) {
	return e.colors ?? [{ format: e.format ?? "rgba8unorm" }];
}
function Ra(e) {
	return e.depth === !0 ? "depth24plus" : e.depth || void 0;
}
function za(e) {
	let t = e.msaa;
	if (t === !0 || t === 4) return 4;
	if (t === void 0 || t === !1) return 1;
	let n = ft();
	throw n.code = "VGPU-TARGET-MSAA-INVALID", n.message = `msaa received ${t}; WebGPU 1|4; use true`, n;
}
function Ba(e, t) {
	if (!e?.size) throw ft();
	let n = Ra(e);
	if (n === "stencil8") throw dt(n);
	if (za(e) === 4) for (let n of La(e)) Va(n.format, t);
}
function Va(e, t) {
	if (t.isCompatibilityMode && e === "rgba16float") throw M("target", "Dawn compatibility mode does not support rgba16float+msaa.", "Use rgba8unorm for MSAA here, or disable msaa.");
}
function Ha(e, t, n, r) {
	let i = {
		view: (t ?? e).createView(),
		resolveTarget: t ? e.createView() : void 0,
		loadOp: r ? "load" : "clear",
		storeOp: t ? "discard" : "store"
	};
	return r || (i.clearValue = Ga(n)), i;
}
function Ua(e, t, n, r, i) {
	if (i) {
		let t = {
			view: e.createView(),
			depthReadOnly: !0
		};
		return Wa(e.format) && (t.stencilReadOnly = !0), t;
	}
	let a = {
		view: e.createView(),
		depthLoadOp: t ? "load" : "clear",
		depthStoreOp: e.sampleCount > 1 ? "discard" : "store"
	};
	return t || (a.depthClearValue = n ?? 1), e.format && Wa(e.format) && (a.stencilLoadOp = t ? "load" : "clear", a.stencilStoreOp = e.sampleCount > 1 ? "discard" : "store", t || (a.stencilClearValue = r ?? 0)), a;
}
function Wa(e) {
	return !!e && e.includes("stencil");
}
function Ga(e) {
	return Array.isArray(e) ? {
		r: e[0],
		g: e[1],
		b: e[2],
		a: e[3]
	} : e;
}
function Ka(e, t) {
	return e[0] === t[0] && e[1] === t[1];
}
function qa(e) {
	return typeof e == "object" && !!e && typeof e.renderPassDescriptor == "function";
}
//#endregion
//#region node_modules/vgpu/dist/pipeline-store.js
var Ja = 1, Ya = 1, Xa = /* @__PURE__ */ new WeakMap(), Za = /* @__PURE__ */ new WeakMap();
function Qa(e) {
	return qa(e) ? {
		colors: e.colors.map((e) => e.format),
		depth: e.depth?.format,
		sampleCount: e.sampleCount
	} : typeof e != "object" || !e ? { colors: [] } : {
		colors: Array.isArray(e.colors) ? [...e.colors] : e.colors ?? [],
		depth: e.depth,
		sampleCount: e.sampleCount ?? 1
	};
}
function $a(e) {
	return `${e.colors.join(",")}:${e.depth ?? "none"}:${e.sampleCount ?? 1}`;
}
function eo(e, t) {
	if (!Array.isArray(e.colors) || e.colors.length === 0) throw ut(t, "colors must be a non-empty array.");
	let n = e.colors.find((e) => typeof e != "string" || e.length === 0);
	if (n !== void 0) throw ut(t, `colors must contain only GPUTextureFormat strings; received ${String(n)}.`);
	if (e.depth !== void 0 && (typeof e.depth != "string" || e.depth.length === 0)) throw ut(t, "depth must be a GPUTextureFormat string.");
	let r = e.sampleCount ?? 1;
	if (r !== 1 && r !== 4) throw ut(t, `sampleCount must be 1 or 4; received ${String(r)}.`);
}
function to(e) {
	let t = `${ho(Xa, e.module, () => Ja++)}|${ho(Za, e.pipelineLayout, () => Ya++)}|${go(e.vertexBufferLayouts ?? [])}|${$a(e.signature)}`, n = e.topology || e.stripIndexFormat ? `${t}|${e.topology ?? "triangle-list"}|${e.stripIndexFormat ?? "none"}` : t, r = e.cullMode || e.frontFace ? `${n}|${e.cullMode ?? "none"}|${e.frontFace ?? "ccw"}` : n, i = e.unclippedDepth ? `${r}|unclipped` : r, a = e.depthKey ? `${i}|${e.depthKey}` : i, o = e.stencilKey ? `${a}|${e.stencilKey}` : a, s = e.multisampleKey ? `${o}|${e.multisampleKey}` : o, c = e.constantsKey ? `${s}|${e.constantsKey}` : s, l = e.entryKey ? `${c}|${e.entryKey}` : c;
	return e.fragmentKey ? `${l}|${e.fragmentKey}` : l;
}
function no(e, t, n, r, i) {
	if (r === void 0) return t.find((e) => e.stage === n);
	if (typeof r != "string") throw Ge(e, `${n} received ${co(r)}; expected an entry point name string.`, i);
	let a = t.find((e) => e.name === r);
	if (!a) throw Ge(e, `"${r}" matches no entry point in the shader; available entry points: ${ro(t)}.`, i);
	if (a.stage !== n) throw Ge(e, `"${r}" is a @${a.stage} entry point, not @${n}; available entry points: ${ro(t)}.`, i);
	return a;
}
function ro(e) {
	return e.length ? e.map((e) => `"${e.name}" (@${e.stage})`).join(", ") : "none";
}
function io(e, t, n, r) {
	if (t !== void 0 && (typeof t != "object" || !t || Array.isArray(t))) throw We(e, `received ${co(t)}; expected { overrideNameOrId: number | boolean }.`, r);
	let i = new Map(n.map((e) => [ao(e), e])), a = {};
	for (let [o, s] of Object.entries(t ?? {})) {
		if (!i.has(o)) throw We(e, `"${o}" matches no override in the shader; available overrides: ${oo(n)}.`, r);
		if (typeof s == "boolean") {
			a[o] = +!!s;
			continue;
		}
		if (typeof s != "number" || !Number.isFinite(s)) throw We(e, `"${o}" received ${co(s)}; use a finite number or a boolean (WebGPU converts the value to the override's WGSL type, and NaN/Infinity fail that conversion).`, r);
		a[o] = s;
	}
	for (let t of n) {
		let n = ao(t);
		if (t.defaultValue === void 0 && !(n in a)) throw We(e, `override '${t.name}' has no default value and must be provided; add constants: { "${n}": value }.`, r);
	}
	return Object.keys(a).length === 0 ? {} : {
		constants: a,
		constantsKey: so(a)
	};
}
function ao(e) {
	return e.id === void 0 ? e.name : String(e.id);
}
function oo(e) {
	return e.length ? e.map((e) => e.id === void 0 ? `"${e.name}"` : `"${e.id}" (@id of ${e.name})`).join(", ") : "none";
}
function so(e) {
	return `cn~${Object.entries(e).sort(([e], [t]) => e < t ? -1 : +(e > t)).map(([e, t]) => `${e}=${t}`).join("~")}`;
}
function co(e) {
	if (typeof e == "string") return `"${e}"`;
	try {
		return JSON.stringify(e) ?? String(e);
	} catch {
		return String(e);
	}
}
function lo(e) {
	let t = /* @__PURE__ */ new Map();
	return {
		get(n, r) {
			let i = t.get(n);
			return i || (i = e.gpu.createShaderModule({
				label: r,
				code: n
			}), t.set(n, i)), i;
		},
		dispose() {
			t.clear();
		}
	};
}
function uo(e) {
	let t = /* @__PURE__ */ new Map();
	return {
		get(n) {
			let r = _o(n), i = t.get(r);
			return i || (i = e.gpu.createPipelineLayout({ bindGroupLayouts: vo(n) }), t.set(r, i)), i;
		},
		dispose() {
			t.clear();
		}
	};
}
function fo(e, t = {}) {
	return new po(e, t);
}
var po = class {
	device;
	#e = /* @__PURE__ */ new Map();
	#t = /* @__PURE__ */ new Set();
	#n;
	#r;
	#i = !1;
	constructor(e, t) {
		this.device = e, this.#n = t.errorSink ?? (() => void 0), this.#r = t.registerSettledSource?.(() => [...this.#t]);
	}
	getReady(e) {
		return this.#e.get(e)?.pipeline;
	}
	getSync(e, t, n) {
		this.#c(n.where);
		let r = this.#e.get(e);
		if (r?.pipeline) return r.pipeline;
		let i = r ?? {};
		r || this.#e.set(e, i);
		let a = this.#a(e, i, t, n);
		if (!a) {
			i.pending || this.#e.delete(e);
			return;
		}
		return i.pipeline = a, i.pending?.resolve(a), i.pending = void 0, a;
	}
	getAsync(e, t, n) {
		this.#c(n.where);
		let r = this.#e.get(e);
		if (r?.pipeline) return Promise.resolve(r.pipeline);
		if (r?.pending) return r.pending.promise;
		let i = {}, a = mo();
		i.pending = a, this.#e.set(e, i);
		let o;
		try {
			o = t();
		} catch (t) {
			let r = ct(n.where, t, n.signature);
			return a.reject(r), this.#e.delete(e), a.promise;
		}
		return this.#l(o), o.then((t) => {
			this.#e.get(e) !== i || i.pipeline || i.pending !== a || (i.pipeline = t, i.pending = void 0, a.resolve(t));
		}, (t) => {
			this.#e.get(e) !== i || i.pipeline || i.pending !== a || (i.pending = void 0, this.#e.delete(e), a.reject(ct(n.where, t, n.signature)));
		}), a.promise;
	}
	dispose() {
		if (this.#i) return;
		this.#i = !0;
		let e = lt("gpu.dispose");
		for (let t of this.#e.values()) t.pending?.reject(e);
		this.#e.clear(), this.#t.clear(), this.#r?.();
	}
	#a(e, t, n, r) {
		let i = this.device.gpu, a = typeof i.pushErrorScope == "function" && typeof i.popErrorScope == "function";
		a && i.pushErrorScope("validation");
		try {
			let i = n();
			return a && this.#o(e, t, r), i;
		} catch (e) {
			a && this.#s();
			let t = ct(r.where, e, r.signature);
			this.#n(t);
			return;
		}
	}
	#o(e, t, n) {
		let r = this.device.gpu.popErrorScope().then((r) => {
			if (!r) return;
			let i = ct(n.where, r, n.signature);
			return this.#e.get(e) === t && this.#e.delete(e), this.#n(i);
		}, (r) => {
			let i = ct(n.where, r, n.signature);
			return this.#e.get(e) === t && this.#e.delete(e), this.#n(i);
		});
		this.#l(r);
	}
	#s() {
		let e = this.device.gpu.popErrorScope?.();
		e && e.catch(() => void 0);
	}
	#c(e) {
		if (this.#i) throw lt(e);
	}
	#l(e) {
		this.#t.add(e), e.catch(() => void 0).then(() => this.#t.delete(e), () => this.#t.delete(e));
	}
};
function mo() {
	let e, t, n = new Promise((n, r) => {
		e = n, t = r;
	});
	return n.catch(() => void 0), {
		promise: n,
		resolve: e,
		reject: t
	};
}
function ho(e, t, n) {
	let r = e.get(t);
	return r || (r = n(), e.set(t, r)), r;
}
function go(e) {
	return JSON.stringify(e.map((e) => ({
		arrayStride: e.arrayStride,
		stepMode: e.stepMode ?? "vertex",
		attributes: [...e.attributes].map((e) => ({
			shaderLocation: e.shaderLocation,
			offset: e.offset,
			format: e.format
		}))
	})));
}
function _o(e) {
	return JSON.stringify([...e.entries()].map(([e, t]) => ({
		group: e,
		entries: bo(t)
	})));
}
function vo(e) {
	let t = Math.max(-1, ...e.keys()), n = [];
	for (let r = 0; r <= t; r++) n.push(yo(e, r));
	return n;
}
function yo(e, t) {
	let n = e.get(t);
	if (!n) throw st(t);
	return n;
}
function bo(e) {
	return (Ee(e)?.entries ?? []).map((e) => ({
		binding: e.binding,
		visibility: e.visibility,
		buffer: e.buffer ? { ...e.buffer } : void 0,
		sampler: e.sampler ? { ...e.sampler } : void 0,
		texture: e.texture ? { ...e.texture } : void 0,
		storageTexture: e.storageTexture ? { ...e.storageTexture } : void 0,
		externalTexture: e.externalTexture ? { ...e.externalTexture } : void 0
	}));
}
//#endregion
//#region node_modules/vgpu/dist/frame-state.js
var xo = kt("frame-state");
function So(e) {
	return e.service(xo, Co);
}
function Co() {
	let e = /* @__PURE__ */ new Set(), t = wo(), n = !1, r = !1, i = {
		time: 0,
		deltaTime: 0,
		frameCount: 0,
		advanceBy(e) {
			i.deltaTime = e, i.time += e, r = !0;
		},
		tick() {
			if (n) throw bt();
			n = !0;
			try {
				let n = wo();
				r ? r = !1 : (i.deltaTime = Math.max(0, (n - t) / 1e3), i.time += i.deltaTime), t = n, i.frameCount += 1;
				for (let t of [...e]) t();
			} finally {
				n = !1;
			}
		},
		onAdvance(t) {
			return e.add(t), () => {
				e.delete(t);
			};
		}
	};
	return i;
}
function wo() {
	return globalThis.performance?.now?.() ?? Date.now();
}
//#endregion
//#region node_modules/vgpu/dist/surface.js
function To(e, t, n = {}) {
	let r = Vt(e, "surface"), i = Do(r), a = i.get(t);
	if (a && !a.disposed) throw ht(a.label);
	let o = new Fo(r.device, t, n, (e) => {
		i.get(e.canvas) === e && i.delete(e.canvas), s(), c();
	}), s = So(r).onAdvance(() => o.applyAutoResize()), c = r.own("resource", () => o.dispose());
	return i.set(t, o), o;
}
var Eo = kt("surfaces");
function Do(e) {
	return e.service(Eo, () => /* @__PURE__ */ new Map());
}
var Oo = 0, ko = 0;
function Ao() {
	return Oo > 0;
}
function jo() {
	return ko > 0;
}
function Mo() {
	ko += 1;
}
function No() {
	--ko;
}
function Po(e) {
	return e instanceof Fo;
}
var Fo = class {
	device;
	canvas;
	options;
	unregister;
	resourceIdentity = m("render-target");
	label;
	context;
	autoResize;
	layoutBacked;
	format;
	#e = new h();
	#t = /* @__PURE__ */ new Set();
	#n = /* @__PURE__ */ new Set();
	#r;
	#i;
	#a = !1;
	#o = !1;
	constructor(e, t, n, r) {
		this.device = e, this.canvas = t, this.options = n, this.unregister = r, this.label = n.label, this.#i = n.clearColor === void 0 ? Pa : Fa(n.clearColor, "surface.clearColor");
		let i = t.getContext("webgpu");
		if (!i) throw mt();
		if (this.context = i, this.layoutBacked = Io(t), n.autoResize === !0 && !this.layoutBacked) throw _t();
		this.autoResize = n.autoResize ?? (!n.size && this.layoutBacked), this.#r = Ho(n.dpr), this.format = n.format ?? Uo();
		let a = Lo(t, n, this.layoutBacked, this.#r);
		(n.size || this.layoutBacked) && Bo(t, a), i.configure({
			device: e.gpu,
			format: this.format,
			alphaMode: n.alphaMode ?? "premultiplied",
			colorSpace: n.colorSpace ?? "srgb",
			usage: Wo()
		});
	}
	get gpu() {
		return this.context;
	}
	get size() {
		return this.#d(), zo(this.canvas);
	}
	get texelSize() {
		let e = this.size;
		return [1 / e[0], 1 / e[1]];
	}
	get color() {
		return this.#d(), new pe(this.device, this.context.getCurrentTexture(), {
			size: this.size,
			format: this.format,
			usage: [
				"render_attachment",
				"texture_binding",
				"copy_src"
			],
			label: this.options.label ? `${this.options.label}.color` : "surface.color"
		}, "external");
	}
	get colors() {
		return [this.color];
	}
	get depth() {
		this.#d();
	}
	get sampleCount() {
		return this.#d(), 1;
	}
	get dpr() {
		return this.#r;
	}
	get clearColor() {
		return Ia(this.#i);
	}
	set clearColor(e) {
		this.#i = Fa(e, "surface.clearColor");
	}
	get disposed() {
		return this.#a;
	}
	resize(e) {
		if (this.#d(), this.#o) throw vt(this.options.label);
		this.#s(Vo(e), this.#r, !0);
	}
	applyAutoResize() {
		if (this.#a || !this.autoResize || !this.layoutBacked) return;
		let e = Ho(this.options.dpr), t = Ro(this.canvas, e);
		this.#s(t, e, !0);
	}
	onResize(e) {
		this.#d(), this.#t.add(e), this.#o = !0, Oo += 1;
		try {
			e(this.#u());
		} finally {
			--Oo, this.#o = !1;
		}
		return () => {
			this.#t.delete(e);
		};
	}
	async read() {
		return this.#d(), this.color.read();
	}
	async readFloats() {
		return this.#d(), this.color.readFloats();
	}
	onDestroy(e) {
		return this.#d(), this.#e.onDestroy(this, e);
	}
	onTexturesRecreated(e) {
		return this.#d(), this.#n.add(e), () => {
			this.#n.delete(e);
		};
	}
	renderPassDescriptor(e = {}) {
		let { clear: t = [
			0,
			0,
			0,
			1
		], preserve: n } = e;
		this.#d();
		let r = {
			view: this.context.getCurrentTexture().createView(),
			loadOp: n ? "load" : "clear",
			storeOp: "store"
		};
		return n || (r.clearValue = Ga(t)), { colorAttachments: [r] };
	}
	dispose() {
		if (!this.#a) {
			this.#a = !0;
			try {
				this.context.unconfigure?.();
			} catch {}
			this.unregister(this), this.#t.clear(), this.#n.clear(), this.#e.emit(this);
		}
	}
	#s(e, t, n) {
		let r = !Ka(zo(this.canvas), e);
		this.#r = t, r && (Bo(this.canvas, e), this.#c(), n && this.#l());
	}
	#c() {
		for (let e of [...this.#n]) e();
	}
	#l() {
		this.#o = !0, Oo += 1;
		try {
			let e = this.#u();
			for (let t of [...this.#t]) t(e);
		} finally {
			--Oo, this.#o = !1;
		}
	}
	#u() {
		let e = zo(this.canvas);
		return {
			width: e[0],
			height: e[1],
			dpr: this.#r,
			surface: this
		};
	}
	#d() {
		if (this.#a) throw gt(this.options.label);
	}
};
function Io(e) {
	return typeof e.clientWidth == "number";
}
function Lo(e, t, n, r) {
	return t.size ? Vo(t.size) : n ? Ro(e, r) : Vo(zo(e));
}
function Ro(e, t) {
	let n = e;
	return Vo([Math.round(n.clientWidth * t), Math.round(n.clientHeight * t)]);
}
function zo(e) {
	let t = e;
	return [t.width, t.height];
}
function Bo(e, t) {
	let n = e;
	n.width = t[0], n.height = t[1];
}
function Vo(e) {
	return [Math.max(1, Math.floor(e[0])), Math.max(1, Math.floor(e[1]))];
}
function Ho(e) {
	let t = globalThis.devicePixelRatio ?? 1;
	return Array.isArray(e) ? Math.min(e[1], Math.max(e[0], t)) : typeof e == "number" ? e : t;
}
function Uo() {
	return globalThis.navigator?.gpu?.getPreferredCanvasFormat?.() ?? "bgra8unorm";
}
function Wo() {
	let e = globalThis.GPUTextureUsage;
	return e ? e.RENDER_ATTACHMENT | e.TEXTURE_BINDING | e.COPY_SRC : void 0;
}
//#endregion
//#region node_modules/vgpu/dist/indirect.js
var Go = {
	drawIndirect: {
		bytes: 16,
		args: "4 u32 values: vertexCount, instanceCount, firstVertex, firstInstance"
	},
	drawIndexedIndirect: {
		bytes: 20,
		args: "5 32-bit values: indexCount, instanceCount, firstIndex, baseVertex (signed), firstInstance"
	},
	dispatchWorkgroupsIndirect: {
		bytes: 12,
		args: "3 u32 values: workgroupCountX, workgroupCountY, workgroupCountZ"
	}
};
function Ko(e, t, n, r) {
	let i = typeof n == "object" && n ? n.buffer : void 0, a = qo(n) ? n : qo(i) ? i : void 0;
	if (!a) throw Ke(e, `received ${Jo(n)}; expected a StorageBuffer or { buffer, offset? }.`, t);
	let o = a === n ? 0 : n.offset ?? 0;
	if (typeof o != "number" || !Number.isInteger(o) || o < 0) throw Ke(e, `offset must be an integer >= 0; received ${Jo(o)}.`, t);
	if (o % 4 != 0) throw Ke(e, `offset must be a multiple of 4 (WebGPU requires "indirectOffset is a multiple of 4"); received ${o}.`, t);
	if (!a.buffer.options.usage.includes("indirect")) throw Ke(e, `the buffer lacks the "indirect" usage (WebGPU requires "indirectBuffer.usage contains INDIRECT"); create it with storage(gpu, ${a.size}, { indirect: true }).`, t);
	let { bytes: s, args: c } = Go[r];
	if (o + s > a.size) throw Ke(e, `${r} reads ${s} bytes (${c}) at offset ${o}, but offset + ${s} = ${o + s} exceeds the buffer size ${a.size}.`, t);
	return {
		buffer: a.gpu,
		offset: o
	};
}
function qo(e) {
	return typeof e == "object" && !!e && "gpu" in e && "size" in e && e.buffer instanceof g;
}
function Jo(e) {
	if (typeof e == "string") return `"${e}"`;
	try {
		return JSON.stringify(e) ?? String(e);
	} catch {
		return String(e);
	}
}
//#endregion
//#region node_modules/vgpu/dist/frame-protocols.js
var Yo = Symbol("vgpu.frame.drawable");
function Xo(e) {
	return e?.[Yo];
}
var Zo = Symbol("vgpu.frame.bundle");
function Qo(e) {
	return e?.[Zo];
}
var $o = Symbol("vgpu.frame.passAttachment");
function es(e) {
	return typeof e?.[$o] == "function" ? e : void 0;
}
//#endregion
//#region node_modules/vgpu/dist/sampler.js
var ts = 1;
function ns(e) {
	let t = /* @__PURE__ */ new Map(), n = /* @__PURE__ */ new WeakMap();
	return {
		sampler(r = {}) {
			let i = rs(r), a = t.get(i);
			return a || (a = e.gpu.createSampler(r), t.set(i, a), n.set(a, {
				kind: "sampler",
				id: ts++
			})), a;
		},
		identity(e) {
			let t = n.get(e);
			return t || (t = {
				kind: "sampler",
				id: ts++
			}, n.set(e, t)), t;
		}
	};
}
function rs(e) {
	if (typeof e != "object" || !e) return JSON.stringify(e);
	if (Array.isArray(e)) return `[${e.map(rs).join(",")}]`;
	let t = e;
	return `{${Object.keys(t).sort().map((e) => `${JSON.stringify(e)}:${rs(t[e])}`).join(",")}}`;
}
//#endregion
//#region node_modules/vgpu/dist/render-service.js
var is = kt("render-service");
function as(e) {
	return e.service(is, os);
}
function os(e) {
	let t = e.device, n = vi(), r = fo(t, {
		errorSink: (t) => e.reportError(t),
		registerSettledSource: (t) => e.registerSettledSource(t)
	}), i = lo(t), a = uo(t), o = ns(t);
	return e.own("service", () => {
		r.dispose(), i.dispose(), a.dispose(), n.dispose();
	}), {
		binds: n,
		pipelines: r,
		shaderModules: i,
		pipelineLayouts: a,
		sampler: (e) => o.sampler(e)
	};
}
//#endregion
//#region node_modules/vgpu/dist/shader-source.js
function ss(e) {
	if (typeof e == "string") return e;
	if (!cs(e) || !("version" in e) || e.version !== 1) throw wt(e);
	let t = e.wgsl;
	if (typeof t != "string") throw wt(e);
	return t;
}
function cs(e) {
	return typeof e == "object" && !!e;
}
//#endregion
//#region node_modules/vgpu/dist/draw.js
function ls(e, t) {
	let n = Vt(e, "draw"), r = as(n), i = ss(t.shader);
	return new fs(n.device, i, {
		...t,
		shader: i
	}, r.binds, void 0, r.pipelines, r.shaderModules, r.pipelineLayouts, (e) => n.reportError(e), (e) => {
		n.trackDelivery(e);
	});
}
var us = 1, ds = /* @__PURE__ */ new WeakMap(), fs = class {
	source;
	label;
	#e = /* @__PURE__ */ new Map();
	constructor(e, t, n, r = vi(), i, a = fo(e), o = lo(e), s = uo(e), c, l) {
		this.source = t, N(e, "Draw.constructor"), this.label = n.label ?? "draw";
		let u = us++, d = _i(t, `${this.label}.wgsl`), f = ks(this.label, n.entry), p = no(this.label, d.entryPoints, "vertex", f.vertex, "draw"), m = no(this.label, d.entryPoints, "fragment", f.fragment, "draw"), h = As(d, p, m), g = [p, m].filter((e) => !!e), _ = fa(d.bindings, g);
		ps(e, this.label, d.bindings, g, _);
		let v = n.geometry, y = p ? bi(p, "inputs", this.label) : [], b = v && Bt in v ? v[Bt](y, `${this.label}.geometry`) : v?.vertexBufferLayouts, x = new Map(ma(e, this.label, d, _)), S = s.get(x), C = o.get(t, `${this.label}.shader`), ee = oc(), te = bs(this.label, n), ne = Es(this.label, n, te), re = js(e, this.label, n), ie = Bs(e, this.label, n), w = Ws(this.label, n), ae = Xs(this.label, n), oe = io(this.label, n.constants, d.overrides, "draw"), se = ba({
			device: e,
			label: this.label,
			drawId: u,
			reflection: d,
			bindGroupLayouts: x,
			cache: r,
			onIdentityChange: (e) => ee.markStale({
				kind: "binding-identity",
				drawLabel: this.label,
				...e
			})
		});
		ds.set(this, {
			id: u,
			device: e,
			opts: n,
			vertexBufferLayouts: b,
			cache: r,
			defaultTarget: i,
			reflection: d,
			visibility: _,
			vertexEntry: p?.name ?? "vs_main",
			fragmentEntry: m?.name ?? "fs_main",
			entryKey: h,
			setCore: se,
			bindGroupLayouts: x,
			pipelineLayout: S,
			shaderModule: C,
			pipelineStore: a,
			pipelineLayouts: s,
			errorSink: c,
			trackSettled: l,
			resolvedPipelineKeys: /* @__PURE__ */ new Set(),
			recordedIn: ee,
			...te,
			...ne,
			...re,
			...ie,
			...w,
			...ae,
			...oe
		}), n.set && this.set(n.set);
		for (let e of n.targets ?? []) this.compileSync(e);
	}
	get gpu() {
		let e = Z(this);
		for (let t of e.resolvedPipelineKeys) {
			let n = e.pipelineStore.getReady(t);
			if (n) return n;
		}
	}
	get targets() {
		return Z(this).opts.targets;
	}
	get [Yo]() {
		return this;
	}
	writesDepth() {
		return nc(this);
	}
	stencilWritingOps() {
		return rc(this);
	}
	set(e) {
		let t = Z(this);
		N(t.device, `${this.label}.set`);
		for (let n of t.setCore.set(e)) t.recordedIn.markStale({
			kind: "binding-identity",
			drawLabel: this.label,
			...n
		});
		return this;
	}
	group(e, t) {
		let n = Z(this);
		N(n.device, `${this.label}.group`);
		let r = this.#e.get(e) ?? this.layout(e), i = n.setCore.claimGroup(e, t, r);
		return n.recordedIn.markStale({
			kind: "group-claim",
			drawLabel: this.label,
			group: e,
			previousIdentity: i,
			newIdentity: `claimed-group:${e}`
		}), this;
	}
	layout(e, t = {}) {
		return N(Z(this).device, `${this.label}.layout`), t.dynamicOffsets ? this.#t(e) : Z(this).setCore.layout(e);
	}
	#t(e) {
		let t = Z(this);
		t.setCore.layout(e);
		let n = this.#e.get(e);
		if (n) return n;
		let r = cc(this, e), i = ga(t.device, `${this.label}.group${e}.dynamic.bgl`, r);
		return this.#e.set(e, i), t.bindGroupLayouts.set(e, i), t.pipelineLayout = t.pipelineLayouts.get(t.bindGroupLayouts), i;
	}
	draw(e = {}) {
		N(Z(this).device, `${this.label}.draw`);
		let t = qa(e) ? { target: e } : e, n = Z(this), r = t.target ?? n.defaultTarget;
		if (!r) throw it(`${this.label}.draw`);
		uc(r, `${this.label}.draw`);
		let i = n.device.gpu.createCommandEncoder(), a = i.beginRenderPass(r.renderPassDescriptor()), o = [];
		try {
			this.encode(a, r, t, (e) => o.push(e));
		} catch (e) {
			q(o), Ti(n.device);
			try {
				a.end();
			} catch {}
			throw e;
		}
		Ni(n.device, a, o, o[0]?.context);
		let s, c = o[0]?.context;
		c && Si(n.device, c);
		try {
			s = i.finish();
		} catch (e) {
			let t = c ? K(n.device) : void 0;
			q(o), t && q([t]);
			let r = t?.context ?? c;
			if (r) {
				ac(n, r.label, r.group, e);
				return;
			}
			throw e;
		}
		if (c) {
			let e = K(n.device);
			e && (o[0] = o[0] ? Oi(e, o[0]) : e);
		}
		let l = o[0]?.context;
		l && Si(n.device, l);
		try {
			n.device.gpu.queue.submit([s]);
		} catch (e) {
			let t = l ? K(n.device) : void 0;
			q(o), t && q([t]);
			let r = t?.context ?? l;
			if (r) {
				ac(n, r.label, r.group, e);
				return;
			}
			throw e;
		}
		if (l) {
			let e = K(n.device);
			e && (o[0] = o[0] ? Oi(e, o[0]) : e);
		}
		if (o.length) {
			let e = Di(n.device, o, { errorSink: n.errorSink });
			n.trackSettled?.(e);
		}
	}
	encode(e, t, n = {}, r) {
		N(Z(this).device, `${this.label}.encode`);
		let i = this.pipelineFor(t, !0);
		if (!i) return;
		e.setPipeline(i);
		let a = Z(this);
		a.blendConstant && e.setBlendConstant(a.blendConstant), a.stencilRef !== void 0 && e.setStencilReference(a.stencilRef);
		for (let t of a.setCore.bindGroups()) this.#n(e, t, n, r);
		this.#o(e, n);
	}
	#n(e, t, n, r) {
		let i = sc(n.offsets, t.group, t.offsets);
		if (!t.claimValidation || !r) {
			e.setBindGroup(t.group, t.bindGroup, i);
			return;
		}
		Si(Z(this).device, t.claimValidation);
		try {
			e.setBindGroup(t.group, t.bindGroup, i);
		} catch (e) {
			throw wi(Z(this).device), D(t.claimValidation.label, t.claimValidation.group, e);
		}
		let a = K(Z(this).device);
		a && r(a);
	}
	compile(e) {
		N(Z(this).device, `${this.label}.compile`);
		let { key: t, signature: n, signatureKey: r } = this.#r(e, `${this.label}.compile`);
		return Z(this).pipelineStore.getAsync(t, () => this.#l(n), {
			where: `${this.label}.compile`,
			signature: r
		}).then(() => (N(Z(this).device, `${this.label}.compile`), Z(this).resolvedPipelineKeys.add(t), this));
	}
	compileSync(e) {
		N(Z(this).device, `${this.label}.compileSync`);
		let { key: t, signature: n, signatureKey: r } = this.#r(e, `${this.label}.compileSync`);
		return Z(this).pipelineStore.getSync(t, () => this.#c(n), {
			where: `${this.label}.compileSync`,
			signature: r
		}) && Z(this).resolvedPipelineKeys.add(t), this;
	}
	pipelineFor(e, t = !1) {
		N(Z(this).device, `${this.label}.pipelineFor`);
		let { key: n, signature: r, signatureKey: i } = this.#r(e, `${this.label}.pipelineFor`, t), a = Z(this).pipelineStore.getSync(n, () => this.#c(r), {
			where: `${this.label}.pipelineFor`,
			signature: i
		});
		return a && Z(this).resolvedPipelineKeys.add(n), a;
	}
	pipelineForAsync(e) {
		N(Z(this).device, `${this.label}.pipelineForAsync`);
		let { key: t, signature: n, signatureKey: r } = this.#r(e, `${this.label}.pipelineForAsync`);
		return Z(this).pipelineStore.getAsync(t, () => this.#l(n), {
			where: `${this.label}.pipelineForAsync`,
			signature: r
		}).then((e) => (N(Z(this).device, `${this.label}.pipelineForAsync`), Z(this).resolvedPipelineKeys.add(t), e));
	}
	#r(e, t, n = !1) {
		let r = this.#i(e, t, n);
		return {
			signature: r,
			signatureKey: $a(r),
			key: this.#a(r)
		};
	}
	#i(e, t, n = !1) {
		let r = Z(this), i = e ?? r.defaultTarget;
		if (!i) throw it(t);
		n || uc(i, t);
		let a = Qa(i);
		if (eo(a, t), r.colorStates && r.colorStates.length !== a.colors.length) throw Re(this.label, `expected one entry per color attachment; colors has ${r.colorStates.length}, but the target signature has ${a.colors.length}.`, t);
		if (r.multisampleState?.alphaToCoverageEnabled && (a.sampleCount ?? 1) <= 1) throw Ue(this.label, `alphaToCoverage requires a multisampled target, but the target signature has sampleCount ${a.sampleCount ?? 1}; create the target with msaa: true.`, t);
		if ((r.stencilState || r.stencilRef !== void 0) && !Wa(a.depth)) throw He(this.label, `stencil requires a depth format with a stencil aspect, but the target signature has ${a.depth ? `"${a.depth}"` : "no depth"}; create the target with depth: "depth24plus-stencil8".`, t);
		return a;
	}
	#a(e) {
		let t = Z(this), n = t.opts.geometry;
		return to({
			module: t.shaderModule,
			pipelineLayout: t.pipelineLayout,
			vertexBufferLayouts: t.vertexBufferLayouts,
			signature: e,
			fragmentKey: t.fragmentKey,
			topology: n?.topology,
			stripIndexFormat: _s(n),
			cullMode: t.cullMode,
			frontFace: t.frontFace,
			unclippedDepth: t.unclippedDepth,
			depthKey: t.depthKey,
			stencilKey: t.stencilKey,
			multisampleKey: t.multisampleKey,
			constantsKey: t.constantsKey,
			entryKey: t.entryKey
		});
	}
	#o(e, t = {}) {
		let n = Z(this).opts.geometry;
		if (n?.vertexBuffers && n.vertexBuffers.forEach((t, n) => e.setVertexBuffer(n, t)), t.indirect !== void 0) return this.#s(e, n, t);
		let r = gs(this.label, n, Z(this).opts, t);
		if (!n?.indexBuffer) return e.draw(r.vertexCount, r.instanceCount, r.firstVertex, r.firstInstance);
		e.setIndexBuffer(n.indexBuffer, n.indexFormat ?? "uint32"), e.drawIndexed(r.indexCount, r.instanceCount, r.firstIndex, r.baseVertex, r.firstInstance);
	}
	#s(e, t, n) {
		let r = `${this.label}.draw`, i = ms.find((e) => n[e] !== void 0);
		if (i !== void 0) throw Ke(this.label, `indirect cannot be combined with ${i} in the same call; the GPU reads the draw arguments from the buffer, so the CPU-side value would be ignored.`, r);
		let a = !!t?.indexBuffer, { buffer: o, offset: s } = Ko(this.label, r, n.indirect, a ? "drawIndexedIndirect" : "drawIndirect");
		if (!a) return e.drawIndirect(o, s);
		e.setIndexBuffer(t.indexBuffer, t.indexFormat ?? "uint32"), e.drawIndexedIndirect(o, s);
	}
	#c(e) {
		let t = Z(this);
		return t.device.gpu.createRenderPipeline({
			label: `${this.label}.pipeline`,
			layout: t.pipelineLayout,
			vertex: {
				module: t.shaderModule,
				entryPoint: t.vertexEntry,
				buffers: [...t.vertexBufferLayouts ?? []],
				...t.constants ? { constants: t.constants } : {}
			},
			fragment: {
				module: t.shaderModule,
				entryPoint: t.fragmentEntry,
				targets: hs(e, t),
				...t.constants ? { constants: t.constants } : {}
			},
			primitive: vs(t.opts.geometry, t.cullMode, t.frontFace, t.unclippedDepth),
			depthStencil: zs(e, t),
			multisample: Ys(e, t)
		});
	}
	#l(e) {
		let t = Z(this);
		return t.device.gpu.createRenderPipelineAsync({
			label: `${this.label}.pipeline`,
			layout: t.pipelineLayout,
			vertex: {
				module: t.shaderModule,
				entryPoint: t.vertexEntry,
				buffers: [...t.vertexBufferLayouts ?? []],
				...t.constants ? { constants: t.constants } : {}
			},
			fragment: {
				module: t.shaderModule,
				entryPoint: t.fragmentEntry,
				targets: hs(e, t),
				...t.constants ? { constants: t.constants } : {}
			},
			primitive: vs(t.opts.geometry, t.cullMode, t.frontFace, t.unclippedDepth),
			depthStencil: zs(e, t),
			multisample: Ys(e, t)
		});
	}
};
function ps(e, t, n, r, i) {
	let a = e.limits;
	for (let [e, o, s] of [[
		"vertex",
		1,
		"maxStorageBuffersInVertexStage"
	], [
		"fragment",
		2,
		"maxStorageBuffersInFragmentStage"
	]]) {
		let c = r.find((t) => t.stage === e);
		if (!c) continue;
		let l = n.filter((e) => e.bindingLayout?.kind === "buffer" && e.bindingLayout.buffer.type !== "uniform" && i(e) & o), u = a[s] ?? a.maxStorageBuffersPerShaderStage;
		if (u !== void 0 && l.length > u) throw ke(t, e, c.name, l.length, u, l);
	}
}
var ms = [
	"vertices",
	"indices",
	"instances",
	"firstVertex",
	"firstIndex",
	"baseVertex",
	"firstInstance"
];
function hs(e, t) {
	return e.colors.map((e, n) => {
		let r = t.colorStates?.[n], i = r?.blendState ?? t.blendState, a = r?.writeMask ?? t.writeMask, o = { format: e };
		return i && (o.blend = i), a !== void 0 && (o.writeMask = a), o;
	});
}
function gs(e, t, n, r) {
	Y(e, "DrawOptions.instances", n.instances), Y(e, "DrawOptions.vertices", n.vertices), Y(e, "DrawOptions.firstInstance", n.firstInstance), Y(e, "DrawCallOptions.instances", r.instances), J(e, "DrawCallOptions.vertices", r.vertices), J(e, "DrawCallOptions.indices", r.indices), J(e, "DrawCallOptions.firstVertex", r.firstVertex), J(e, "DrawCallOptions.firstIndex", r.firstIndex), J(e, "DrawCallOptions.baseVertex", r.baseVertex), Y(e, "DrawCallOptions.firstInstance", r.firstInstance), Y(e, "GeometryLike.vertexCount", t?.vertexCount), Y(e, "GeometryLike.indexCount", t?.indexCount), Y(e, "GeometryLike.instanceCount", t?.instanceCount), J(e, "GeometryLike.firstVertex", t?.firstVertex), J(e, "GeometryLike.firstIndex", t?.firstIndex), J(e, "GeometryLike.baseVertex", t?.baseVertex);
	let i = !!t?.indexBuffer, a = t?.geometry ?? (t && Bt in t ? t : void 0), o = r.firstVertex ?? t?.firstVertex ?? 0, s = r.vertices ?? t?.vertexCount ?? n.vertices ?? 3, c = r.firstIndex ?? t?.firstIndex ?? 0, l = r.indices ?? t?.indexCount ?? 0, u = r.baseVertex ?? t?.baseVertex ?? 0;
	if (i) ys(e, "index", c, l, a?.indexCount);
	else if (r.indices !== void 0 || r.firstIndex !== void 0 || r.baseVertex !== void 0) throw ot(`${e}.draw`, "Index range needs an indexed geometry.");
	return i || ys(e, "vertex", o, s, a?.vertexCount), {
		instanceCount: r.instances ?? n.instances ?? t?.instanceCount ?? 1,
		firstInstance: r.firstInstance ?? n.firstInstance ?? 0,
		vertexCount: s,
		firstVertex: o,
		indexCount: l,
		firstIndex: c,
		baseVertex: u
	};
}
function _s(e) {
	let t = e?.topology ?? "triangle-list";
	return e?.stripIndexFormat ?? (t.endsWith("strip") ? e?.indexFormat : void 0);
}
function vs(e, t, n, r) {
	let i = e?.topology ?? "triangle-list", a = _s(e), o = a ? {
		topology: i,
		stripIndexFormat: a
	} : { topology: i };
	return t !== void 0 && (o.cullMode = t), n !== void 0 && (o.frontFace = n), r && (o.unclippedDepth = !0), o;
}
function ys(e, t, n, r, i) {
	if (!(i === void 0 || n + r <= i)) throw ot(`${e}.draw`, `${t} range [${n}, ${n + r}) exceeds parent geometry ${t} count ${i}.`);
}
function J(e, t, n) {
	if (!(n === void 0 || Number.isInteger(n) && n >= 0)) throw ot(`${e}.draw`, `${t} must be an integer >= 0; received ${String(n)}.`);
}
function Y(e, t, n) {
	if (n !== void 0 && !(Number.isInteger(n) && n >= 0)) throw new E({
		code: "VGPU-R1-DRAW-COUNT",
		message: `${t} of '${e}' must be an integer >= 0; received ${String(n)}. Use 0 only when you want to issue a valid draw with no vertices/instances.`,
		where: `${e}.draw`
	});
}
function bs(e, t) {
	let n = t.blend === void 0 ? void 0 : Ss(e, t.blend), r = t.writeMask === void 0 ? void 0 : Qs(e, t.writeMask), i = t.colors === void 0 ? void 0 : xs(e, t.colors);
	return {
		blendState: n,
		writeMask: r,
		colorStates: i,
		fragmentKey: i ? `${$s(n, r)}@${i.map(tc).join("@")}` : n || r !== void 0 ? $s(n, r) : void 0
	};
}
function xs(e, t) {
	if (!Array.isArray(t)) throw Re(e, `colors must be an array; received ${X(t)}.`);
	return t.map((t, n) => {
		if (t == null) return null;
		if (typeof t != "object" || Array.isArray(t)) throw Re(e, `colors[${n}] must be null or { blend?, writeMask? }; received ${X(t)}.`);
		let r = t.blend === void 0 ? void 0 : Ss(`${e}.colors[${n}]`, t.blend), i = t.writeMask === void 0 ? void 0 : Qs(`${e}.colors[${n}]`, t.writeMask);
		return !r && i === void 0 ? null : {
			blendState: r,
			writeMask: i
		};
	});
}
function Ss(e, t) {
	if (t === "alpha") return ws({
		src: "src-alpha",
		dst: "one-minus-src-alpha"
	}, {
		src: "one",
		dst: "one-minus-src-alpha"
	});
	if (t === "premultiplied") return ws({
		src: "one",
		dst: "one-minus-src-alpha"
	}, {
		src: "one",
		dst: "one-minus-src-alpha"
	});
	if (t === "additive") return ws({
		src: "one",
		dst: "one"
	}, {
		src: "one",
		dst: "one"
	});
	if (typeof t != "object" || !t || !Cs(t.color)) throw Fe(e, t);
	let n = t.color, r = t.alpha;
	if (r !== void 0 && !Cs(r)) throw Fe(e, t);
	return ws(n, r ?? n);
}
function Cs(e) {
	return typeof e == "object" && !!e && typeof e.src == "string" && typeof e.dst == "string";
}
function ws(e, t) {
	return {
		color: Ts(e),
		alpha: Ts(t)
	};
}
function Ts(e) {
	return {
		srcFactor: e.src,
		dstFactor: e.dst,
		operation: e.op ?? "add"
	};
}
function Es(e, t, n) {
	if (t.blendConstant === void 0) return {};
	let r = t.blendConstant;
	if (!Array.isArray(r) || r.length !== 4 || r.some((e) => typeof e != "number" || !Number.isFinite(e))) throw Ie(e, `received ${X(r)}; expected [r, g, b, a] finite numbers.`);
	if (!Ds(n).some((e) => e && Os(e))) throw Ie(e, "no color target's effective blend uses a \"constant\"/\"one-minus-constant\" factor (colors[i].blend replaces the top-level blend for that target), so blendConstant would have no effect.");
	return { blendConstant: {
		r: r[0],
		g: r[1],
		b: r[2],
		a: r[3]
	} };
}
function Ds(e) {
	return e.colorStates ? e.colorStates.map((t) => t?.blendState ?? e.blendState) : [e.blendState];
}
function Os(e) {
	return [
		e.color.srcFactor,
		e.color.dstFactor,
		e.alpha.srcFactor,
		e.alpha.dstFactor
	].some((e) => e === "constant" || e === "one-minus-constant");
}
function ks(e, t) {
	if (t === void 0) return {};
	if (typeof t != "object" || !t || Array.isArray(t)) throw Ge(e, `received ${X(t)}; expected { vertex?, fragment? } entry point names.`);
	return t;
}
function As(e, t, n) {
	let r = e.entryPoints.find((e) => e.stage === "vertex"), i = e.entryPoints.find((e) => e.stage === "fragment");
	if (t !== r || n !== i) return `en~${t?.name ?? ""}~${n?.name ?? ""}`;
}
function js(e, t, n) {
	return {
		cullMode: n.cull === void 0 ? void 0 : Ns(t, n.cull),
		frontFace: n.frontFace === void 0 ? void 0 : Ps(t, n.frontFace),
		unclippedDepth: n.unclippedDepth === void 0 ? void 0 : Ms(e, t, n.unclippedDepth)
	};
}
function Ms(e, t, n) {
	if (typeof n != "boolean") throw Ve(t, `received ${X(n)}; expected a boolean.`);
	if (n) {
		if (!e.features.has("depth-clip-control")) throw Ve(t, "the device lacks the \"depth-clip-control\" feature; request it at init: init({ requiredFeatures: [\"depth-clip-control\"] }) on an adapter that supports it.");
		return !0;
	}
}
function Ns(e, t) {
	if (t === "none" || t === "front" || t === "back") return t;
	throw ze(e, t);
}
function Ps(e, t) {
	if (t === "ccw" || t === "cw") return t;
	throw Be(e, t);
}
var Fs = {
	depthWriteEnabled: !0,
	depthCompare: "less-equal"
}, Is = [
	"never",
	"less",
	"equal",
	"less-equal",
	"greater",
	"not-equal",
	"greater-equal",
	"always"
], Ls = -2147483648, Rs = 2147483647;
function zs(e, t) {
	if (e.depth) return {
		format: e.depth,
		...t.depthState ?? Fs,
		...t.stencilState ?? {}
	};
}
function Bs(e, t, n) {
	if (n.depth === void 0) return {};
	let r = Vs(e, t, n.depth, n.geometry?.topology ?? "triangle-list");
	return {
		depthState: r,
		depthKey: Hs(r)
	};
}
function Vs(e, t, n, r) {
	if (n === !1) return {
		depthWriteEnabled: !1,
		depthCompare: "always"
	};
	if (typeof n != "object" || !n) throw O(t, `received ${X(n)}.`);
	if (n.write !== void 0 && typeof n.write != "boolean") throw O(t, `write must be a boolean; received ${X(n.write)}.`);
	if (n.compare !== void 0 && !Is.includes(n.compare)) throw O(t, `compare must be a GPUCompareFunction; received ${X(n.compare)}.`);
	if (n.bias !== void 0 && !Number.isInteger(n.bias)) throw O(t, `bias must be an integer (WebGPU depthBias is i32); received ${X(n.bias)}.`);
	if (n.bias !== void 0 && (n.bias < Ls || n.bias > Rs)) throw O(t, `bias must fit in the i32 range [${Ls}, ${Rs}] (WebGPU depthBias is i32); received ${X(n.bias)}.`);
	if (n.biasSlopeScale !== void 0 && !Number.isFinite(n.biasSlopeScale)) throw O(t, `biasSlopeScale must be a finite number; received ${X(n.biasSlopeScale)}.`);
	if (n.biasClamp !== void 0 && !Number.isFinite(n.biasClamp)) throw O(t, `biasClamp must be a finite number; received ${X(n.biasClamp)}.`);
	let i = n.bias ?? 0, a = n.biasSlopeScale ?? 0, o = n.biasClamp ?? 0;
	if ((i !== 0 || a !== 0 || o !== 0) && !r.startsWith("triangle")) throw O(t, `bias, biasSlopeScale, and biasClamp must be 0 for "${r}" topology.`);
	if (o !== 0 && e.isCompatibilityMode) throw O(t, `biasClamp must be 0 on a compatibility-mode device; received ${X(n.biasClamp)}.`);
	return {
		depthWriteEnabled: n.write ?? !0,
		depthCompare: n.compare ?? "less-equal",
		...i === 0 ? {} : { depthBias: i },
		...a === 0 ? {} : { depthBiasSlopeScale: a },
		...o === 0 ? {} : { depthBiasClamp: o }
	};
}
function Hs(e) {
	return `${+!!e.depthWriteEnabled}~${e.depthCompare}~${e.depthBias ?? 0}~${e.depthBiasSlopeScale ?? 0}~${e.depthBiasClamp ?? 0}`;
}
var Us = [
	"keep",
	"zero",
	"replace",
	"invert",
	"increment-clamp",
	"decrement-clamp",
	"increment-wrap",
	"decrement-wrap"
];
function Ws(e, t) {
	if (t.stencil === void 0) return {};
	let n = t.stencil;
	if (typeof n != "object" || !n || Array.isArray(n)) throw He(e, `received ${X(n)}; expected { front?, back?, readMask?, writeMask?, ref? }.`);
	let r = n.front === void 0 ? void 0 : Gs(e, "front", n.front), i = n.back === void 0 ? void 0 : Gs(e, "back", n.back);
	Ks(e, "readMask", n.readMask), Ks(e, "writeMask", n.writeMask), Ks(e, "ref", n.ref);
	let a = {
		...r ? { stencilFront: r } : {},
		...i ?? r ? { stencilBack: i ?? { ...r } } : {},
		...n.readMask === void 0 ? {} : { stencilReadMask: n.readMask },
		...n.writeMask === void 0 ? {} : { stencilWriteMask: n.writeMask }
	}, o = a.stencilFront !== void 0 || a.stencilBack !== void 0 || a.stencilReadMask !== void 0 || a.stencilWriteMask !== void 0;
	return !o && n.ref === void 0 ? {} : {
		...o ? {
			stencilState: a,
			stencilKey: qs(a)
		} : {},
		...n.ref === void 0 ? {} : { stencilRef: n.ref }
	};
}
function Gs(e, t, n) {
	if (typeof n != "object" || !n || Array.isArray(n)) throw He(e, `${t} must be a { compare?, fail?, depthFail?, pass? } object; received ${X(n)}.`);
	if (n.compare !== void 0 && !Is.includes(n.compare)) throw He(e, `${t}.compare must be a GPUCompareFunction; received ${X(n.compare)}.`);
	for (let [r, i] of [
		["fail", n.fail],
		["depthFail", n.depthFail],
		["pass", n.pass]
	]) if (i !== void 0 && !Us.includes(i)) throw He(e, `${t}.${r} must be a GPUStencilOperation; received ${X(i)}.`);
	return {
		compare: n.compare ?? "always",
		failOp: n.fail ?? "keep",
		depthFailOp: n.depthFail ?? "keep",
		passOp: n.pass ?? "keep"
	};
}
function Ks(e, t, n) {
	if (n !== void 0 && (typeof n != "number" || !Number.isInteger(n) || n < 0 || n > 4294967295)) throw He(e, `${t} must be an integer in [0, 0xFFFFFFFF] (WebGPU GPUStencilValue is u32); received ${X(n)}.`);
}
function qs(e) {
	return `st~${Js(e.stencilFront)}~${Js(e.stencilBack)}~${e.stencilReadMask ?? 4294967295}~${e.stencilWriteMask ?? 4294967295}`;
}
function Js(e) {
	return e ? `${e.compare},${e.failOp},${e.depthFailOp},${e.passOp}` : "default";
}
function Ys(e, t) {
	return {
		count: e.sampleCount ?? 1,
		...t.multisampleState ?? {}
	};
}
function Xs(e, t) {
	if (t.multisample === void 0) return {};
	let n = t.multisample;
	if (typeof n != "object" || !n || Array.isArray(n)) throw Ue(e, `received ${X(n)}; expected { alphaToCoverage?, mask? }.`);
	if (n.alphaToCoverage !== void 0 && typeof n.alphaToCoverage != "boolean") throw Ue(e, `alphaToCoverage must be a boolean; received ${X(n.alphaToCoverage)}.`);
	if (n.mask !== void 0 && (typeof n.mask != "number" || !Number.isInteger(n.mask) || n.mask < 0 || n.mask > 4294967295)) throw Ue(e, `mask must be an integer in [0, 0xFFFFFFFF] (WebGPU GPUSampleMask is u32); received ${X(n.mask)}.`);
	let r = {
		...n.alphaToCoverage === void 0 ? {} : { alphaToCoverageEnabled: n.alphaToCoverage },
		...n.mask === void 0 ? {} : { mask: n.mask }
	};
	return r.alphaToCoverageEnabled === void 0 && r.mask === void 0 ? {} : {
		multisampleState: r,
		multisampleKey: Zs(r)
	};
}
function Zs(e) {
	return `ms~${+!!e.alphaToCoverageEnabled}~${e.mask ?? 4294967295}`;
}
function Qs(e, t) {
	if (!Array.isArray(t)) throw Le(e, X(t));
	let n = 0;
	for (let r of t) if (r === "r") n |= 1;
	else if (r === "g") n |= 2;
	else if (r === "b") n |= 4;
	else if (r === "a") n |= 8;
	else throw Le(e, X(r));
	return n;
}
function $s(e, t) {
	return `${ec(e)};${t ?? 15}`;
}
function ec(e) {
	if (!e) return "none;none";
	let t = e.color, n = e.alpha;
	return `${t.srcFactor},${t.dstFactor},${t.operation};${n.srcFactor},${n.dstFactor},${n.operation}`;
}
function tc(e) {
	return e ? `${e.blendState ? ec(e.blendState) : "inherit"};${e.writeMask ?? "inherit"}` : "inherit";
}
function X(e) {
	if (typeof e == "string") return `"${e}"`;
	try {
		return JSON.stringify(e) ?? String(e);
	} catch {
		return String(e);
	}
}
function nc(e) {
	return (Z(e).depthState ?? Fs).depthWriteEnabled;
}
function rc(e) {
	let t = Z(e), n = t.stencilState;
	if (!n || n.stencilWriteMask === 0) return [];
	let r = t.cullMode ?? "none", i = [], a = (e, t) => {
		if (t) for (let [n, r] of [
			["fail", t.failOp],
			["depthFail", t.depthFailOp],
			["pass", t.passOp]
		]) r !== void 0 && r !== "keep" && i.push(`${e}.${n}: "${r}"`);
	};
	return r !== "front" && a("front", n.stencilFront), r !== "back" && a("back", n.stencilBack), i;
}
function ic(e, t, n, r = {}, i) {
	e.encode(t, n, r, i);
}
function Z(e) {
	let t = ds.get(e);
	if (!t) throw TypeError("Invalid Draw instance");
	return t;
}
function ac(e, t, n, r) {
	let i = (async () => {
		await Ei(e.device), N(e.device, `${t}.validation`);
		let i = D(t, n, r);
		e.errorSink ? await e.errorSink(i) : console.error(i);
	})();
	return e.trackSettled?.(i), i;
}
function oc() {
	let e = /* @__PURE__ */ new Set();
	return {
		add(t) {
			e.add(t);
		},
		delete(t) {
			e.delete(t);
		},
		list() {
			return [...e];
		},
		markStale(t) {
			for (let n of e) n.markStale(t);
		}
	};
}
function sc(e, t, n) {
	return e ? Array.isArray(e) ? e : e[t] ?? n : n;
}
function cc(e, t) {
	let n = Z(e);
	return pa(n.reflection.bindings, t, n.visibility).map(lc);
}
function lc(e) {
	return e.buffer ? {
		...e,
		buffer: {
			...e.buffer,
			hasDynamicOffset: !0
		}
	} : e;
}
function uc(e, t) {
	if (Po(e) && !jo()) throw pt(t);
}
//#endregion
//#region node_modules/vgpu/dist/effect.js
function dc(e, t, n = {}) {
	if ("geometry" in n) throw M("effect", "effect() never accepts vertex buffers; use draw(gpu, { shader, geometry: geometry(gpu, descriptor) }).");
	let r = Vt(e, "effect"), i = as(r);
	return new pc(r.device, ss(t), n, i.binds, void 0, i.pipelines, i.shaderModules, i.pipelineLayouts, (e) => r.reportError(e), (e) => {
		r.trackDelivery(e);
	});
}
var fc = /* @__PURE__ */ new WeakMap(), pc = class {
	get gpu() {
		return Q(this).gpu;
	}
	constructor(e, t, n = {}, r, i, a, o, s, c, l) {
		let u = mc(t), d = new fs(e, u, {
			shader: u,
			set: n.set,
			label: n.label ?? "effect",
			blend: n.blend,
			writeMask: n.writeMask
		}, r, i, a, o, s, c, l);
		fc.set(this, d);
	}
	set(e) {
		return Q(this).set(e), this;
	}
	draw(e = {}) {
		Q(this).draw(qa(e) ? { target: e } : e);
	}
	compile(e) {
		return Q(this).compile(e).then(() => this);
	}
	compileSync(e) {
		return Q(this).compileSync(e), this;
	}
	encode(e, t, n = {}, r) {
		ic(Q(this), e, t, n, r);
	}
	get [Yo]() {
		return Q(this)[Yo];
	}
};
function Q(e) {
	let t = fc.get(e);
	if (!t) throw TypeError("Invalid Effect instance");
	return t;
}
function mc(e) {
	return hc(e) ? e : `
struct VgpuFullscreenVertexOut {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
};
@vertex fn vgpu_fullscreen_vs(@builtin(vertex_index) vi: u32) -> VgpuFullscreenVertexOut {
  var pos = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  var uv = array<vec2f, 3>(vec2f(0.0, 1.0), vec2f(2.0, 1.0), vec2f(0.0, -1.0));
  var out: VgpuFullscreenVertexOut;
  out.position = vec4f(pos[vi], 0.0, 1.0);
  out.uv = uv[vi];
  return out;
}
${e}`;
}
function hc(e) {
	return _i(e, "effect.wgsl").entryPoints.some((e) => e.stage === "vertex");
}
//#endregion
//#region node_modules/vgpu/dist/frame.js
function gc(e, t, n = {}) {
	return vc(Vt(e, "frameLoop")).loop(t, n);
}
var _c = kt("frame-runner");
function vc(e) {
	return e.service(_c, (e) => {
		let t = So(e);
		return new kc(() => {
			let t = () => {}, n = new yc(e.device, void 0, (t) => e.reportError(t), (t) => {
				e.trackDelivery(t);
			}, () => t());
			return t = e.own("scheduler", () => n.cancel()), n;
		}, () => t.tick(), (t) => e.own("scheduler", () => t.stop()));
	});
}
var yc = class {
	device;
	defaultTarget;
	errorSink;
	trackSettled;
	releaseLifecycle;
	done = Promise.resolve();
	#e;
	#t = [];
	#n = /* @__PURE__ */ new Set();
	#r = /* @__PURE__ */ new Set();
	#i = !1;
	#a = !1;
	#o = !1;
	constructor(e, t, n, r, i) {
		this.device = e, this.defaultTarget = t, this.errorSink = n, this.trackSettled = r, this.releaseLifecycle = i, N(e, "Frame.constructor"), this.#e = e.gpu.createCommandEncoder({ label: "vgpu.frame" });
	}
	pass(e, t) {
		if (this.#a) throw xt("Frame.pass");
		N(this.device, "Frame.pass");
		let n = qa(e), r = typeof t == "function" ? t : (e) => e.draw(t), i = n ? e : e.target ?? this.defaultTarget;
		if (!i) throw it("Frame.pass");
		if (Po(i) && this.#i) throw pt("Frame.pass");
		let a = n ? void 0 : e.clear, o = a === !1;
		if (o && i.sampleCount === 4) throw qe();
		let s = n ? void 0 : e.clearDepth;
		if (s !== void 0) {
			if (typeof s != "number" || !(s >= 0 && s <= 1)) throw Je(s);
			if (o) throw Xe();
			if (!i.depth) throw Je(s, "but the target has no depth attachment, so clearDepth would have no effect.", "Create the target with depth: true (or a depth format), or drop clearDepth.");
		}
		let c = n ? void 0 : e.clearStencil;
		if (c !== void 0) {
			if (typeof c != "number" || !Number.isInteger(c) || c < 0 || c > 4294967295) throw Ze(`received ${String(c)}; expected an integer in [0, 0xFFFFFFFF] (WebGPU GPUStencilValue).`);
			if (o) throw Qe();
			let e = i.depth?.format;
			if (!Wa(e)) throw Ze(`received ${String(c)}, but the target's depth format ${e ? `"${e}"` : "(none)"} has no stencil aspect, so clearStencil would have no effect.`);
		}
		let l = n ? void 0 : e.depthReadOnly;
		if (l !== void 0 && typeof l != "boolean") throw A(`received ${$(l)}; expected a boolean.`, "Pass depthReadOnly: true to open the pass with a read-only depth attachment, or omit it.");
		if (l) {
			if (!i.depth) throw A("is set, but the target has no depth attachment, so there is nothing to make read-only.", "Create the target with depth: true (or a depth format), or drop depthReadOnly.");
			if (i.sampleCount === 4) throw $e();
			if (s !== void 0) throw A("cannot be combined with clearDepth; a read-only depth aspect omits its load/store ops and is never cleared.", "Remove clearDepth, or drop depthReadOnly.");
			if (c !== void 0) throw A("cannot be combined with clearStencil; a read-only stencil aspect omits its load/store ops and is never cleared.", "Remove clearStencil, or drop depthReadOnly.");
		}
		let u = n ? void 0 : Ec(e.viewport, this.device.gpu.limits, i.size), d = n ? void 0 : Dc(e.scissor, i.size), f = [], p;
		try {
			let t = n || e.timer === void 0 ? void 0 : this.#d(e.timer, i, f, wc), m = (n || e.visibility === void 0 ? void 0 : this.#d(e.visibility, i, f, Tc))?.occlusion, h = i.renderPassDescriptor({
				clear: a === void 0 || a === !0 || a === !1 ? i.clearColor ?? Pa : a,
				preserve: o,
				clearDepth: s,
				clearStencil: c,
				depthReadOnly: l
			});
			t?.timestampWrites && (h = {
				...h,
				timestampWrites: t.timestampWrites
			}), m && (h = {
				...h,
				occlusionQuerySet: m.querySet
			}), p = this.#e.beginRenderPass(h), u && p.setViewport(u.x, u.y, u.width, u.height, u.minDepth, u.maxDepth), d && p.setScissorRect(d[0], d[1], d[2], d[3]), this.#o = !0;
			try {
				r(new bc(p, i, this.#t, l === !0, m, this, (e) => {
					if (N(this.device, e), this.#a) throw xt(e);
				}));
			} finally {
				this.#o = !1;
			}
		} catch (e) {
			this.#l(f), q(this.#t), this.#t.length = 0, Ti(this.device);
			try {
				p?.end();
			} catch {}
			throw e;
		}
		Ni(this.device, p, this.#t);
	}
	submit() {
		if (this.#i || this.#a) return;
		N(this.device, "Frame.submit"), this.#i = !0, this.releaseLifecycle?.();
		for (let e of this.#u()) e.finalizeFrame(this, this.#e);
		let e, t = this.#t[0]?.context;
		t && Si(this.device, t);
		try {
			e = this.#e.finish();
		} catch (e) {
			this.#s(this.#c());
			let n = t ? K(this.device) : void 0;
			q(this.#t), n && q([n]);
			let r = n?.context ?? t;
			if (!r) throw e;
			this.done = this.#p(this.#f(r.label, r.group, e));
			return;
		}
		if (t) {
			let e = K(this.device);
			e && (this.#t[0] = this.#t[0] ? Oi(e, this.#t[0]) : e);
		}
		let n = this.#t[0]?.context;
		n && Si(this.device, n);
		try {
			this.device.gpu.queue.submit([e]);
		} catch (e) {
			this.#s(this.#c());
			let t = n ? K(this.device) : void 0;
			q(this.#t), t && q([t]);
			let r = t?.context ?? n;
			if (!r) throw e;
			this.done = this.#p(this.#f(r.label, r.group, e));
			return;
		}
		if (n) {
			let e = K(this.device);
			e && (this.#t[0] = this.#t[0] ? Oi(e, this.#t[0]) : e);
		}
		for (let e of this.#u()) e.frameSubmitted(this);
		this.#s(this.#r), this.done = this.#p(Di(this.device, this.#t, { errorSink: this.errorSink }));
	}
	cancel() {
		if (!this.#a) {
			if (this.#i) throw Ct("Frame.cancel");
			if (this.#o) throw St("Frame.cancel");
			this.#a = !0, this.releaseLifecycle?.(), this.#s(this.#c()), this.#n.clear(), this.#r.clear(), q(this.#t), this.#t.length = 0;
		}
	}
	#s(e) {
		for (let t of [...e]) t.frameAbandoned(this);
	}
	#c() {
		return [...this.#n, ...this.#r];
	}
	#l(e) {
		for (let t of [...e]) this.#n.delete(t), this.#r.add(t);
	}
	#u() {
		return [...this.#n].filter((e) => !this.#r.has(e));
	}
	#d(e, t, n, r) {
		let i = es(e);
		if (!i) throw r(e);
		let a;
		try {
			a = i[$o]({
				frame: this,
				device: this.device,
				target: t
			});
		} catch (e) {
			throw this.#l(this.#n), e;
		}
		return this.#n.add(a.owner), n.push(a.owner), a;
	}
	async #f(e, t, n) {
		await Ei(this.device), N(this.device, "Frame.validation");
		let r = D(e, t, n);
		this.errorSink ? await this.errorSink(r) : console.error(r);
	}
	#p(e) {
		return this.trackSettled?.(e), e;
	}
}, bc = class {
	encoder;
	target;
	validations;
	depthReadOnly;
	occlusionSource;
	frame;
	assertFrameOpen;
	#e = !1;
	constructor(e, t, n, r = !1, i, a, o) {
		this.encoder = e, this.target = t, this.validations = n, this.depthReadOnly = r, this.occlusionSource = i, this.frame = a, this.assertFrameOpen = o;
	}
	draw(e, t = {}) {
		this.assertFrameOpen?.("FramePass.draw");
		let n = Sc(e);
		this.depthReadOnly && xc(n, this.target), n.encode(this.encoder, this.target, t, (e) => this.validations.push(e));
	}
	occlusion(e, t) {
		if (this.assertFrameOpen?.("FramePass.occlusion"), !this.occlusionSource) throw nt();
		if (this.#e) throw rt();
		let n = this.occlusionSource.beginQuery(e, this.frame);
		this.encoder.beginOcclusionQuery(n), this.#e = !0;
		try {
			typeof t == "function" ? t() : this.draw(t);
		} finally {
			this.#e = !1, this.encoder.endOcclusionQuery();
		}
	}
	bundles(...e) {
		if (this.assertFrameOpen?.("FramePass.bundles"), this.depthReadOnly) throw A("pass cannot replay bundles: bundle records bundles with writable depth/stencil, and WebGPU only executes read-only-recorded bundles in a read-only pass.", "Encode the draws directly with pass.draw(...) inside the depthReadOnly pass.", "FramePass.bundles");
		let t = e.map((e) => Qo(e) ?? Cc());
		for (let e of t) e.assertReplayable(this.target);
		this.encoder.executeBundles(t.map((e) => e.gpu));
	}
};
function xc(e, t) {
	if (e.writesDepth()) throw A(`pass cannot encode draw '${e.label}': its depth state writes depth (the default is write: true). Give the draw depth: { write: false } (or depth: false to disable depth testing).`, "Use depth: { write: false } on the draw, or open the pass without depthReadOnly.", "FramePass.draw");
	if (Wa(t.depth?.format)) {
		let t = e.stencilWritingOps();
		if (t.length) throw A(`pass cannot encode draw '${e.label}': its stencil ops can write (${t.join(", ")}), and the pass's stencil aspect is read-only too.`, "Use \"keep\" for those ops or stencil writeMask: 0, or open the pass without depthReadOnly.", "FramePass.draw");
	}
}
function Sc(e) {
	let t = Xo(e);
	if (!t) throw TypeError("Invalid Effect instance: pass.draw() expects a Draw or an Effect created by this library.");
	return t;
}
function Cc() {
	throw new E({
		code: "VGPU-R3-BUNDLE-INVALID",
		message: "p.bundles() expected bundles created by bundle(gpu, { target }, cb).",
		where: "FramePass.bundles"
	});
}
function wc(e) {
	return et(`FramePassOptions.timer received ${$(e)}; expected a TimerSpan from timer.span(name).`, "Create const passTimer = timer(gpu) once, then pass passTimer.span(\"name\") per pass.", "Frame.pass");
}
function Tc(e) {
	return tt(`FramePassOptions.visibility received ${$(e)}; expected a Visibility from visibility(gpu).`, "Create const vis = visibility(gpu) once, then pass { target, visibility: vis } per pass.", "Frame.pass");
}
function Ec(e, t, n) {
	if (e === void 0) return;
	if (typeof e != "object" || !e || Array.isArray(e)) throw k(`received ${$(e)}; expected { x?, y?, width, height, minDepth?, maxDepth? }.`);
	let { x: r = 0, y: i = 0, width: a, height: o, minDepth: s = 0, maxDepth: c = 1 } = e;
	for (let [e, t] of [
		["x", r],
		["y", i],
		["width", a],
		["height", o],
		["minDepth", s],
		["maxDepth", c]
	]) if (typeof t != "number" || !Number.isFinite(t)) throw k(`${e} received ${$(t)}; expected a finite number.`);
	let l = t.maxTextureDimension2D, u = l * 2, d = `target is ${n[0]}x${n[1]}px, device maxTextureDimension2D is ${l}`;
	if (!(a >= 0 && a <= l)) throw k(`width ${a} is outside [0, ${l}] (${d}).`);
	if (!(o >= 0 && o <= l)) throw k(`height ${o} is outside [0, ${l}] (${d}).`);
	if (!(r >= -u && r + a <= u - 1)) throw k(`x ${r} with width ${a} is outside [${-u}, ${u - 1}] (${d}).`);
	if (!(i >= -u && i + o <= u - 1)) throw k(`y ${i} with height ${o} is outside [${-u}, ${u - 1}] (${d}).`);
	if (!(s >= 0 && s <= 1)) throw k(`minDepth ${s} is outside [0, 1].`);
	if (!(c >= 0 && c <= 1)) throw k(`maxDepth ${c} is outside [0, 1].`);
	if (!(s <= c)) throw k(`minDepth ${s} exceeds maxDepth ${c}.`);
	return {
		x: r,
		y: i,
		width: a,
		height: o,
		minDepth: s,
		maxDepth: c
	};
}
function Dc(e, t) {
	if (e === void 0) return;
	if (!Array.isArray(e) || e.length !== 4) throw Ye(`received ${$(e)}; expected [x, y, width, height].`);
	let [n, r, i, a] = e;
	for (let [e, t] of [
		["x", n],
		["y", r],
		["width", i],
		["height", a]
	]) if (typeof t != "number" || !Number.isInteger(t) || t < 0) throw Ye(`${e} received ${$(t)}; expected a non-negative integer.`);
	let [o, s] = t;
	if (n + i > o || r + a > s) throw Ye(`[${n}, ${r}, ${i}, ${a}] exceeds the target's current size ${o}x${s}px (x + width <= ${o}, y + height <= ${s}).`);
	return [
		n,
		r,
		i,
		a
	];
}
function $(e) {
	return typeof e == "string" ? `'${e}'` : Array.isArray(e) ? `[${e.map((e) => $(e)).join(", ")}]` : typeof e == "object" && e ? "an object" : String(e);
}
function Oc(e) {
	let t = e?.code;
	return t === "VGPU-DEVICE-DISPOSED" || t === "VGPU-DEVICE-LOST";
}
var kc = class {
	createFrame;
	advance;
	trackLoop;
	#e = !1;
	constructor(e, t, n) {
		this.createFrame = e, this.advance = t, this.trackLoop = n;
	}
	frame(e) {
		if (this.#e || Ao()) throw bt();
		this.#e = !0, Mo();
		try {
			this.advance();
			let t = this.createFrame();
			if (e) try {
				e(t);
			} finally {
				try {
					t.submit();
				} catch (e) {
					if (!Oc(e)) throw e;
				}
			}
			return t;
		} finally {
			No(), this.#e = !1;
		}
	}
	loop(e, t = {}) {
		let n = !1, r = globalThis.requestAnimationFrame ?? ((e) => setTimeout(() => e(performance.now()), 16)), i = globalThis.cancelAnimationFrame ?? ((e) => clearTimeout(e)), a = t.fps && t.fps > 0 ? 1e3 / t.fps : 0, o, s = 0, c = (t) => {
			n || (Ac(t, o, a) && (o = t, this.frame(e)), n || (s = r(c)));
		};
		s = r(c);
		let l, u = { stop() {
			n = !0, i(s), l?.(), l = void 0;
		} };
		return l = this.trackLoop?.(u), u;
	}
};
function Ac(e, t, n) {
	return t === void 0 || n <= 0 || e - t >= n;
}
//#endregion
//#region node_modules/vgpu/dist/target-offscreen.js
function jc(e, t) {
	return new Mc(Vt(e, "target").device, t);
}
var Mc = class {
	device;
	options;
	resourceIdentity = m("render-target");
	#e = new h();
	#t = /* @__PURE__ */ new Set();
	#n;
	#r;
	#i;
	#a;
	#o;
	constructor(e, t) {
		this.device = e, this.options = t, Ba(t, e), this.#o = t.clearColor === void 0 ? Pa : Fa(t.clearColor, "target.clearColor"), this.#n = t.size, this.#r = this.#u(), this.#i = this.sampleCount === 4 ? this.#d() : void 0, this.#a = this.#f();
	}
	get gpu() {
		return this.color.gpu;
	}
	get size() {
		return this.#n;
	}
	get texelSize() {
		return [1 / this.#n[0], 1 / this.#n[1]];
	}
	get color() {
		return this.#r[0];
	}
	get colors() {
		return this.#r;
	}
	get depth() {
		return this.#a;
	}
	get format() {
		return La(this.options)[0]?.format ?? "rgba8unorm";
	}
	get clearColor() {
		return Ia(this.#o);
	}
	set clearColor(e) {
		this.#o = Fa(e, "target.clearColor");
	}
	get sampleCount() {
		return za(this.options);
	}
	resize(e) {
		Ka(this.#n, e) || this.#s(e);
	}
	async read() {
		return this.color.read();
	}
	async readFloats() {
		return this.color.readFloats();
	}
	onDestroy(e) {
		return this.#e.onDestroy(this, e);
	}
	onTexturesRecreated(e) {
		return this.#t.add(e), () => {
			this.#t.delete(e);
		};
	}
	destroy() {
		this.#e.emit(this), this.#t.clear(), this.#l();
	}
	renderPassDescriptor(e = {}) {
		let { clear: t = [
			0,
			0,
			0,
			1
		], preserve: n, clearDepth: r, clearStencil: i, depthReadOnly: a } = e;
		return {
			colorAttachments: this.#r.map((e, r) => Ha(e, this.#i?.[r], t, n)),
			depthStencilAttachment: this.#a ? Ua(this.#a, n, r, i, a) : void 0
		};
	}
	#s(e) {
		this.#l(), this.#n = [e[0], e[1]], this.#r = this.#u(), this.#i = this.sampleCount === 4 ? this.#d() : void 0, this.#a = this.#f(), this.#c();
	}
	#c() {
		for (let e of [...this.#t]) e();
	}
	#l() {
		for (let e of this.#r) e.destroy();
		for (let e of this.#i ?? []) e.destroy();
		this.#a?.destroy();
	}
	#u() {
		return La(this.options).map((e, t) => this.device.createTexture({
			size: this.#n,
			format: e.format,
			usage: [
				"render_attachment",
				"texture_binding",
				"copy_src"
			],
			sampleCount: 1,
			label: this.options.label ? `${this.options.label}.color${t}.resolve` : void 0
		}));
	}
	#d() {
		return La(this.options).map((e, t) => this.device.createTexture({
			size: this.#n,
			format: e.format,
			usage: ["render_attachment"],
			sampleCount: 4,
			label: this.options.label ? `${this.options.label}.color${t}` : void 0
		}));
	}
	#f() {
		let e = Ra(this.options);
		return e ? this.device.createTexture({
			size: this.#n,
			format: e,
			usage: ["render_attachment", "texture_binding"],
			sampleCount: this.sampleCount,
			label: this.options.label ? `${this.options.label}.depth` : void 0
		}) : void 0;
	}
};
//#endregion
//#region node_modules/vgpu/dist/storage.js
function Nc(e, t, n = "read-write") {
	let r = Vt(e, "storage"), i = typeof n == "string" ? { access: n } : n, a = Fc(r.device, t, i.access ?? "read-write", void 0, i.indirect ?? !1);
	return Ut(r, a, (e) => e.destroy(), (e) => {
		a.onDestroy(e);
	});
}
var Pc = class e {
	size;
	access;
	buffer;
	constructor(e, t) {
		this.buffer = e, this.access = t, this.size = e.options.size;
	}
	static create(t, n, r, i, a = !1) {
		let o = a ? [
			"storage",
			"copy_dst",
			"copy_src",
			"indirect"
		] : [
			"storage",
			"copy_dst",
			"copy_src"
		], s = t.createBuffer({
			size: n,
			usage: o,
			label: i
		});
		return new e(s, r);
	}
	read() {
		return this.buffer.read(this.size);
	}
	write(e, t = 0) {
		this.buffer.write(Ic(e), t);
	}
	get gpu() {
		return this.buffer.gpu;
	}
	get resourceIdentity() {
		return this.buffer.resourceIdentity;
	}
	onDestroy(e) {
		return this.buffer.onDestroy(e);
	}
	destroy() {
		this.buffer.destroy();
	}
};
function Fc(e, t, n, r, i = !1) {
	return Pc.create(e, t, n, r, i);
}
function Ic(e) {
	if (e instanceof ArrayBuffer || ArrayBuffer.isView(e)) return e;
	throw TypeError("StorageBuffer.write() requires ArrayBuffer or ArrayBufferView.");
}
//#endregion
//#region node_modules/vgpu/dist/index.js
function Lc(e) {
	return Pt("browser", e);
}
//#endregion
//#region web/memory-graph-state.mjs
function Rc(e, { wallNowMs: t = Date.now(), rendererNowSeconds: n = 0 } = {}) {
	let r = new Float32Array(Math.max(1, e.nodes.length) * 12);
	e.nodes.forEach((e, i) => {
		let a = i * 12;
		r.set([
			e.position.x,
			e.position.y,
			e.position.z,
			e.radius,
			...e.color
		], a);
		let o = 0;
		e.selected && (o |= 1), e.correlated && (o |= 2), e.causal && (o |= 4), e.hovered && (o |= 8), e.neighbor && (o |= 16), e.dimmed && (o |= 32), e.liveUntil > t && (o |= 64), e.boundary && (o |= 128);
		let s = e.liveUntil ? n + (e.liveUntil - t) / 1e3 : 0;
		r.set([
			o,
			s,
			0,
			0
		], a + 8);
	});
	let i = new Float32Array(Math.max(1, e.edges.length) * 4);
	return e.edges.forEach((e, t) => {
		let n = e.kind;
		e.highlighted && (n += 2), e.hovered && (n += 4), e.bidirectional && (n += 8), i.set([
			e.source,
			e.target,
			n,
			0
		], t * 4);
	}), {
		nodes: r,
		edges: i,
		nodeCount: e.nodes.length,
		edgeCount: e.edges.length
	};
}
function zc(e, t) {
	return !e && t > 0;
}
//#endregion
//#region web/memory-graph.ts
var Bc = 2e3, Vc = 3e3, Hc = "\nstruct Node { position_radius: vec4f, color: vec4f, state: vec4f }\nstruct Camera { viewport: vec2f, rotation: vec2f, zoom: f32, time: f32, has_selection: f32, reduced_motion: f32 }\n@group(0) @binding(0) var<storage, read> nodes: array<Node>;\n@group(0) @binding(1) var<uniform> camera: Camera;\nstruct Out { @builtin(position) position: vec4f, @location(0) local: vec2f, @location(1) color: vec4f, @location(2) state: vec4f };\nfn project(p: vec3f) -> vec3f {\n  let cy = cos(camera.rotation.x); let sy = sin(camera.rotation.x);\n  let ct = cos(camera.rotation.y); let st = sin(camera.rotation.y);\n  let x = p.x * cy + p.z * sy;\n  let z = -p.x * sy + p.z * cy;\n  let y = p.y * ct - z * st;\n  let z2 = p.y * st + z * ct;\n  let depth = 4.2 + z2;\n  let aspect = max(0.1, camera.viewport.x / camera.viewport.y);\n  return vec3f(x / depth * camera.zoom / aspect, y / depth * camera.zoom, clamp(depth / 9.0, 0.01, 0.99));\n}\n@vertex fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {\n  let corners = array<vec2f, 6>(vec2f(-1,-1), vec2f(1,-1), vec2f(-1,1), vec2f(-1,1), vec2f(1,-1), vec2f(1,1));\n  let node = nodes[ii]; let center = project(node.position_radius.xyz); let corner = corners[vi];\n  var radius = node.position_radius.w;\n  let flags = u32(node.state.x);\n  if ((flags & 64u) != 0u && camera.reduced_motion < 0.5) {\n    let age = max(0.0, 6.0 - (node.state.y - camera.time));\n    radius *= 1.0 + exp(-age * 1.8) * 1.8;\n  }\n  var out: Out;\n  out.position = vec4f(center.xy + corner * radius * 2.0 / camera.viewport, center.z, 1.0);\n  out.local = corner; out.color = node.color; out.state = node.state; return out;\n}\n@fragment fn fs_main(input: Out) -> @location(0) vec4f {\n  let distance = length(input.local);\n  let flags = u32(input.state.x);\n  if ((flags & 128u) != 0u) {\n    let diamond = abs(input.local.x) + abs(input.local.y);\n    let dash = fract((atan2(input.local.y, input.local.x) + 3.14159265) * 2.55);\n    if (diamond > 1.0 || diamond < 0.68 || dash > 0.7) { discard; }\n    var alpha = select(0.26, 0.9, (flags & 4u) != 0u);\n    if ((flags & 32u) != 0u) { alpha = 0.04; }\n    return vec4f(1.0, 0.8, 0.46, alpha);\n  }\n  if (distance > 1.0) { discard; }\n  var alpha = smoothstep(1.0, 0.72, distance);\n  if ((flags & 32u) != 0u) { alpha *= 0.09; }\n  var color = input.color.rgb;\n  if ((flags & 1u) != 0u || (flags & 8u) != 0u) { color = vec3f(1.0); }\n  if ((flags & 2u) != 0u) { color = mix(color, vec3f(0.72, 1.0, 0.8), 0.45); }\n  if ((flags & 4u) != 0u) { color = mix(color, vec3f(1.0), 0.35); }\n  let ring = select(0.0, smoothstep(0.72, 0.82, distance) * (1.0 - smoothstep(0.9, 1.0, distance)), (flags & 1u) != 0u || (flags & 64u) != 0u);\n  return vec4f(color + ring * 0.45, max(alpha, ring));\n}", Uc = "\nstruct Node { position_radius: vec4f, color: vec4f, state: vec4f }\nstruct Edge { data: vec4f }\nstruct Camera { viewport: vec2f, rotation: vec2f, zoom: f32, time: f32, has_selection: f32, reduced_motion: f32 }\n@group(0) @binding(0) var<storage, read> nodes: array<Node>;\n@group(0) @binding(1) var<storage, read> edges: array<Edge>;\n@group(0) @binding(2) var<uniform> camera: Camera;\nstruct Out { @builtin(position) position: vec4f, @location(0) uv: vec2f, @location(1) flags: f32 };\nfn project(p: vec3f) -> vec3f {\n  let cy = cos(camera.rotation.x); let sy = sin(camera.rotation.x);\n  let ct = cos(camera.rotation.y); let st = sin(camera.rotation.y);\n  let x = p.x * cy + p.z * sy; let z = -p.x * sy + p.z * cy;\n  let y = p.y * ct - z * st; let z2 = p.y * st + z * ct; let depth = 4.2 + z2;\n  let aspect = max(0.1, camera.viewport.x / camera.viewport.y);\n  return vec3f(x / depth * camera.zoom / aspect, y / depth * camera.zoom, clamp(depth / 9.0 + 0.001, 0.01, 0.99));\n}\n@vertex fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> Out {\n  let edge = edges[ii].data; let a = project(nodes[u32(edge.x)].position_radius.xyz); let b = project(nodes[u32(edge.y)].position_radius.xyz);\n  var uv = vec2f(0,-1); if (vi == 1u || vi == 4u || vi == 5u) { uv.x = 1; } if (vi == 2u || vi == 3u || vi == 5u) { uv.y = 1; }\n  let point = mix(a.xy, b.xy, uv.x); let delta = (b.xy - a.xy) * camera.viewport; let normal = normalize(vec2f(-delta.y, delta.x));\n  let bits = u32(edge.z); let width = select(4.0, 5.5, (bits & 2u) != 0u || (bits & 4u) != 0u);\n  var out: Out; out.position = vec4f(point + normal * uv.y * width / camera.viewport, mix(a.z,b.z,uv.x), 1); out.uv = uv; out.flags = edge.z; return out;\n}\n@fragment fn fs_main(input: Out) -> @location(0) vec4f {\n  let flags = u32(input.flags); let semantic = (flags & 1u) != 0u; let highlighted = (flags & 2u) != 0u; let hovered = (flags & 4u) != 0u; let bidirectional = (flags & 8u) != 0u;\n  let shaft = abs(input.uv.y) < 0.22;\n  let forward_arrow = input.uv.x > 0.78 && abs(input.uv.y) < (1.0 - input.uv.x) / 0.22;\n  let reverse_arrow = bidirectional && input.uv.x < 0.22 && abs(input.uv.y) < input.uv.x / 0.22;\n  if (!shaft && !forward_arrow && !reverse_arrow) { discard; }\n  if (semantic && shaft && fract(input.uv.x * 12.0) > 0.58) { discard; }\n  var color = select(vec3f(0.33,0.40,0.35), vec3f(0.47,0.71,1.0), semantic);\n  if (highlighted || hovered) { color = vec3f(0.9,1.0,0.93); }\n  var alpha = select(0.26, 0.82, highlighted || hovered); if (camera.has_selection > 0.5 && !highlighted && !hovered) { alpha = 0.035; }\n  return vec4f(color, alpha);\n}", Wc = "\nstruct Params { texel: vec2f }\n@group(0) @binding(0) var src: texture_2d<f32>;\n@group(0) @binding(1) var<uniform> params: Params;\n@fragment fn fs_main(@location(0) uv: vec2f) -> @location(0) vec4f {\n  return textureLoad(src, vec2u(vec2f(uv) / params.texel), 0);\n}";
async function Gc(e, t) {
	if (window.location.hostname !== "localhost" && window.location.hostname !== "127.0.0.1" && window.location.hostname !== "[::1]" || !window.isSecureContext || !("gpu" in navigator)) throw Error("Live Memory Graph requires WebGPU on localhost. Open Habibi at http://localhost:8787 in a WebGPU-capable browser.");
	let n = await Lc({ powerPreference: "high-performance" }), r = To(n, e, { dpr: [1, 2] }), i = jc(n, {
		size: r.size,
		format: "rgba8unorm",
		depth: !0,
		label: "memory-scene"
	}), a = Nc(n, Bc * 48, "read"), o = Nc(n, Vc * 16, "read"), s = ls(n, {
		shader: Hc,
		vertices: 6,
		blend: "alpha",
		depth: {
			write: !0,
			compare: "less-equal"
		},
		label: "memory-nodes"
	}), c = ls(n, {
		shader: Uc,
		vertices: 6,
		blend: "alpha",
		depth: {
			write: !1,
			compare: "less-equal"
		},
		label: "memory-edges"
	}), l = dc(n, Wc, { label: "memory-composite" }), u = 0, d = 0, f = null, p = !1, m = !0, h = !1, g = !1, _ = null, v = .55, y = -.24, b = 1, x = null, S = null, C = null, ee = !1, te = !1, ne = matchMedia("(prefers-reduced-motion: reduce)").matches, re = performance.now(), ie = re, w = () => (performance.now() - re) / 1e3, ae = () => ({
		viewport: [Math.max(1, r.size[0]), Math.max(1, r.size[1])],
		rotation: [v, y],
		zoom: b,
		time: w(),
		has_selection: +!!ee,
		reduced_motion: +!!ne
	}), oe = (e) => {
		let t = Math.max(.1, r.size[0] / Math.max(1, r.size[1])), n = Math.cos(v), i = Math.sin(v), a = Math.cos(y), o = Math.sin(y), s = .1;
		for (let r of e.nodes) {
			let e = r.position, c = e.x * n + e.z * i, l = -e.x * i + e.z * n, u = e.y * a - l * o, d = 4.2 + e.y * o + l * a;
			s = Math.max(s, Math.abs(c / d / t), Math.abs(u / d));
		}
		b = Math.min(4, .82 / s);
	}, se = r.onResize(({ width: e, height: t }) => i.resize([e, t])), ce = gc(n, (e) => {
		if (!m || h || r.size[0] < 1 || r.size[1] < 1) return;
		let t = performance.now(), n = Math.min(64, t - ie);
		if (ie = t, x !== null) {
			let e = ((x - v + Math.PI) % (Math.PI * 2) + Math.PI * 2) % (Math.PI * 2) - Math.PI;
			v += e * .12, y += ((S ?? y) - y) * .12, b += ((C ?? b) - b) * .12, Math.abs(e) < .004 && (x = null, S = null, C = null);
		} else !ne && !te && (v += 18e-5 * n);
		let f = ae();
		s.set({
			nodes: a,
			camera: f
		}), c.set({
			nodes: a,
			edges: o,
			camera: f
		}), e.pass({
			target: i,
			clear: [
				.018,
				.022,
				.04,
				1
			],
			clearDepth: 1
		}, (e) => {
			d && e.draw(c, { instances: d }), u && e.draw(s, { instances: u });
		}), l.set({
			src: i.color,
			params: { texel: i.texelSize }
		}), e.pass({ target: r }, (e) => e.draw(l));
	}), le = () => {}, ue = () => {
		g || (g = !0, m = !1, ce.stop(), se(), le(), r.dispose(), n.dispose());
	}, de = (e) => {
		_ || h || (_ = Error(e), ue(), t.onFatal(e));
	}, T = () => {
		if (_) throw _;
		if (h) throw Error("Memory graph renderer is disposed. Reload to retry.");
	};
	return le = n.onError((e) => de(`WebGPU rendering failed: ${String(e)}`)), n.gpu.lost.then((e) => de(`WebGPU device lost: ${e.message || e.reason}`)), {
		get renderer() {
			return "vgpu/WebGPU";
		},
		setScene(e) {
			T(), f = e, zc(p, e.nodes.length) && (oe(e), p = !0);
			let t = Rc(e, {
				wallNowMs: Date.now(),
				rendererNowSeconds: w()
			});
			if (t.nodeCount > Bc || t.edgeCount > Vc) throw Error("Memory graph exceeds its bounded GPU capacity.");
			a.write(t.nodes), o.write(t.edges), u = t.nodeCount, d = t.edgeCount, ee = e.nodes.some((e) => e.selected), te = e.nodes.some((e) => e.selected || e.hovered);
		},
		setActive(e) {
			e && T(), m = e;
		},
		rotate(e, t) {
			T(), x = null, S = null, C = null, v += e * .0055, y = Math.max(-1.1, Math.min(.35, y - t * .003));
		},
		zoom(e) {
			T(), C = null, b = Math.max(.5, Math.min(8, b * e));
		},
		fit() {
			T(), x = null, S = null, C = null, v = .55, y = -.24, f && oe(f);
		},
		focus(e) {
			T();
			let t = Math.atan2(-e.x, e.z), n = -.18, r = Math.max(b, 4.4);
			ne ? (v = t, y = n, b = r, x = null, S = null, C = null) : (x = t, S = n, C = r);
		},
		project(e) {
			let t = Math.cos(v), n = Math.sin(v), i = Math.cos(y), a = Math.sin(y), o = e.x * t + e.z * n, s = -e.x * n + e.z * t, c = e.y * i - s * a, l = 4.2 + (e.y * a + s * i), u = Math.max(1, r.size[0]), d = Math.max(1, r.size[1]);
			return {
				x: u / 2 + o / l * b / (u / d) * u / 2,
				y: d / 2 - c / l * b * d / 2,
				depth: l
			};
		},
		dispose() {
			h || (h = !0, ue());
		}
	};
}
//#endregion
export { Wc as COMPOSITE_SHADER, Uc as EDGE_SHADER, Hc as NODE_SHADER, Gc as createMemoryGraphRenderer };
