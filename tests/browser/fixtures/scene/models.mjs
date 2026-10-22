// Writes the glTF fixtures for glTF materials and animation into public/models:
// `node tests/browser/fixtures/scene/models.mjs`.
import { writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";
import path from "node:path";

const out = path.join(path.dirname(new URL(import.meta.url).pathname), "public/models");

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc = (b) => {
  let c = 0xffffffff;
  for (const x of b) c = crcTable[(c ^ x) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};

/** An RGB PNG of `w`×`h` pixels from `pixel(x, y) -> [r, g, b]`. */
function png(w, h, pixel) {
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) raw.set(pixel(x, y), y * (w * 3 + 1) + 1 + x * 3);
  const chunk = (type, data) => {
    const b = Buffer.alloc(12 + data.length);
    b.writeUInt32BE(data.length, 0);
    b.write(type, 4, "ascii");
    data.copy(b, 8);
    b.writeUInt32BE(crc(b.subarray(4, 8 + data.length)), 8 + data.length);
    return b;
  };
  const head = Buffer.alloc(13);
  head.writeUInt32BE(w, 0);
  head.writeUInt32BE(h, 4);
  head.set([8, 2, 0, 0, 0], 8);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", head), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

/** A unit box: positions, normals, texture coordinates (each face 0..1) and indices. */
function box() {
  const pos = [], nrm = [], uv = [], idx = [];
  for (const [n, u, v] of [
    [[0, 0, 1], [1, 0, 0], [0, 1, 0]],
    [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
    [[1, 0, 0], [0, 0, -1], [0, 1, 0]],
    [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
    [[0, 1, 0], [1, 0, 0], [0, 0, -1]],
    [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
  ]) {
    const b = pos.length / 3;
    for (const [su, sv] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) {
      pos.push(...[0, 1, 2].map((k) => (n[k] + su * u[k] + sv * v[k]) / 2));
      nrm.push(...n);
      uv.push((su + 1) / 2, (1 - sv) / 2);
    }
    idx.push(b, b + 1, b + 2, b, b + 2, b + 3);
  }
  return { pos, nrm, uv, idx };
}

/** A .glb of one box with `material`, its `images` (PNG buffers), and optional `animations`. */
function glb(file, { material, images = [], animations = [] }) {
  const g = box();
  const chunks = [];
  let length = 0;
  const view = (bytes) => {
    const pad = (4 - (bytes.length % 4)) % 4;
    chunks.push(Buffer.from(bytes), Buffer.alloc(pad));
    const v = { buffer: 0, byteOffset: length, byteLength: bytes.length };
    length += bytes.length + pad;
    return json.bufferViews.push(v) - 1;
  };
  const json = { asset: { version: "2.0" }, scene: 0, scenes: [{ nodes: [0] }], bufferViews: [], accessors: [], buffers: [] };
  const accessor = (values, type, componentType = 5126) => {
    const arr = componentType === 5126 ? new Float32Array(values) : new Uint16Array(values);
    const n = { SCALAR: 1, VEC3: 3, VEC2: 2, VEC4: 4 }[type];
    const a = { bufferView: view(new Uint8Array(arr.buffer)), componentType, count: values.length / n, type };
    if (type === "VEC3" && componentType === 5126) {
      a.min = [0, 1, 2].map((k) => Math.min(...values.filter((_, i) => i % 3 === k)));
      a.max = [0, 1, 2].map((k) => Math.max(...values.filter((_, i) => i % 3 === k)));
    }
    if (type === "SCALAR" && componentType === 5126) {
      a.min = [Math.min(...values)];
      a.max = [Math.max(...values)];
    }
    return json.accessors.push(a) - 1;
  };
  const attributes = { POSITION: accessor(g.pos, "VEC3"), NORMAL: accessor(g.nrm, "VEC3"), TEXCOORD_0: accessor(g.uv, "VEC2") };
  json.meshes = [{ primitives: [{ attributes, indices: accessor(g.idx, "SCALAR", 5123), material: 0 }] }];
  json.nodes = [{ mesh: 0 }];
  json.materials = [material];
  if (images.length) {
    json.images = images.map((b) => ({ bufferView: view(b), mimeType: "image/png" }));
    json.textures = images.map((_, i) => ({ source: i }));
  }
  if (animations.length)
    json.animations = animations.map(({ name, path, times, values, interpolation }) => ({
      name,
      samplers: [{ input: accessor(times, "SCALAR"), output: accessor(values, path === "rotation" ? "VEC4" : "VEC3"), interpolation }],
      channels: [{ sampler: 0, target: { node: 0, path } }],
    }));
  const bin = Buffer.concat(chunks);
  json.buffers = [{ byteLength: bin.length }];
  let text = Buffer.from(JSON.stringify(json));
  text = Buffer.concat([text, Buffer.alloc((4 - (text.length % 4)) % 4, 0x20)]);
  const head = Buffer.alloc(12);
  head.writeUInt32LE(0x46546c67, 0);
  head.writeUInt32LE(2, 4);
  head.writeUInt32LE(12 + 8 + text.length + 8 + bin.length, 8);
  const ch = (type, data) => {
    const h = Buffer.alloc(8);
    h.writeUInt32LE(data.length, 0);
    h.writeUInt32LE(type, 4);
    return Buffer.concat([h, data]);
  };
  writeFileSync(path.join(out, file), Buffer.concat([head, ch(0x4e4f534a, text), ch(0x004e4942, bin)]));
}

const red = [1, 0, 0, 1];
// Glows green without light.
glb("emissive.glb", { material: { pbrMetallicRoughness: { baseColorFactor: [0, 0, 0, 1] }, emissiveFactor: [0, 1, 0] } });
// The same red as metal and as plastic: metal has no diffuse light.
glb("metal.glb", { material: { pbrMetallicRoughness: { baseColorFactor: red, metallicFactor: 1, roughnessFactor: 0.5 } } });
glb("plastic.glb", { material: { pbrMetallicRoughness: { baseColorFactor: red, metallicFactor: 0, roughnessFactor: 0.5 } } });
// Stripes of normals leaning left and right.
const lean = [[64, 128, 230], [192, 128, 230]];
glb("bumpy.glb", {
  material: { pbrMetallicRoughness: { baseColorFactor: [1, 1, 1, 1], metallicFactor: 0 }, normalTexture: { index: 0 } },
  images: [png(16, 16, (x) => lean[Math.floor(x / 4) % 2])],
});
// Ambient light reaches only the right half.
glb("occluded.glb", {
  material: { pbrMetallicRoughness: { baseColorFactor: [1, 1, 1, 1], metallicFactor: 0 }, occlusionTexture: { index: 0 } },
  images: [png(16, 16, (x) => (x < 8 ? [0, 0, 0] : [255, 255, 255]))],
});
// Slides left and right, and turns (cubic spline) on another clip.
glb("moving.glb", {
  material: { pbrMetallicRoughness: { baseColorFactor: [1, 0.5, 0, 1], metallicFactor: 0 } },
  animations: [
    { name: "slide", path: "translation", times: [0, 1, 2], values: [-2, 0, 0, 2, 0, 0, -2, 0, 0], interpolation: "LINEAR" },
    {
      name: "turn",
      path: "rotation",
      times: [0, 2],
      values: [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
      interpolation: "CUBICSPLINE",
    },
  ],
});
