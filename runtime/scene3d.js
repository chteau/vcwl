// 3D scenes (the `scene` element), loaded by the host the first time a page paints one.
// A scene is drawn with WebGL into one offscreen canvas, which the host then copies into the
// page's canvas at the scene's place: the scene stays clipped and layered like any element.

const VS = `
attribute vec3 position;
attribute vec3 normal;
attribute vec2 uv;
uniform mat4 model, viewProj, lightViewProj;
uniform mat3 normalMat;
varying vec3 vNormal, vWorld;
varying vec2 vUv;
varying vec4 vShadow;
void main() {
  vec4 w = model * vec4(position, 1.0);
  vWorld = w.xyz;
  vNormal = normalMat * normal;
  vUv = uv;
  vShadow = lightViewProj * w;
  gl_Position = viewProj * w;
}`;

const FS = `
precision mediump float;
uniform vec3 color, ambient, eye, pickColor, highlight;
uniform float shininess, opacity;
uniform bool pick, textured;
uniform int lightCount, shadowLight;
uniform vec4 lightPos[8];
uniform vec3 lightColor[8];
uniform sampler2D tex, shadowMap;
varying vec3 vNormal, vWorld;
varying vec2 vUv;
varying vec4 vShadow;
float unpack(vec4 c) { return dot(c, vec4(1.0, 1.0 / 255.0, 1.0 / 65025.0, 1.0 / 16581375.0)); }
// Fraction of light reaching this point from the shadow-casting light (3×3 PCF).
float lit(float bias) {
  vec3 p = vShadow.xyz / vShadow.w * 0.5 + 0.5;
  if (p.x < 0.0 || p.x > 1.0 || p.y < 0.0 || p.y > 1.0 || p.z > 1.0) return 1.0;
  float sum = 0.0;
  for (int x = -1; x <= 1; x++)
    for (int y = -1; y <= 1; y++)
      sum += p.z - bias > unpack(texture2D(shadowMap, p.xy + vec2(x, y) / 1024.0)) ? 0.0 : 1.0;
  return sum / 9.0;
}
void main() {
  if (pick) { gl_FragColor = vec4(pickColor, 1.0); return; }
  vec3 base = textured ? color * texture2D(tex, vUv).rgb : color;
  vec3 v = normalize(eye - vWorld);
  // Two-sided: light the side facing the camera, whatever the triangle winding.
  vec3 n = normalize(vNormal);
  if (dot(n, v) < 0.0) n = -n;
  vec3 c = ambient * base;
  for (int i = 0; i < 8; i++) {
    if (i >= lightCount) break;
    vec3 l = lightPos[i].w > 0.5 ? normalize(lightPos[i].xyz - vWorld) : normalize(lightPos[i].xyz);
    float d = max(dot(n, l), 0.0);
    float s = d > 0.0 && shininess > 0.0 ? pow(max(dot(n, normalize(l + v)), 0.0), shininess) * 0.5 : 0.0;
    float k = i == shadowLight ? lit(0.002 + 0.004 * (1.0 - d)) : 1.0;
    c += lightColor[i] * (base * d + vec3(s)) * k;
  }
  c += highlight * 0.35;
  gl_FragColor = vec4(c * opacity, opacity);
}`;

// Depth from the shadow-casting light, packed into RGBA (WebGL 1 has no depth textures
// without an extension).
const SHADOW_VS = `
attribute vec3 position;
uniform mat4 model, lightViewProj;
void main() { gl_Position = lightViewProj * model * vec4(position, 1.0); }`;
const SHADOW_FS = `
precision highp float;
void main() {
  vec4 c = fract(gl_FragCoord.z * vec4(1.0, 255.0, 65025.0, 16581375.0));
  gl_FragColor = c - c.yzww * vec4(1.0 / 255.0, 1.0 / 255.0, 1.0 / 255.0, 0.0);
}`;
const SHADOW_SIZE = 1024;

// ----- matrices (column-major, as WebGL expects) -----

const mul = (a, b) => {
  const o = new Float32Array(16);
  for (let c = 0; c < 4; c++)
    for (let r = 0; r < 4; r++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
      o[c * 4 + r] = s;
    }
  return o;
};
const ident = () => new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);

function perspective(fovDeg, aspect, near, far) {
  const f = 1 / Math.tan((fovDeg * Math.PI) / 360), nf = 1 / (near - far);
  return new Float32Array([f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0]);
}

function ortho(r, near, far) {
  const nf = 1 / (near - far);
  return new Float32Array([1 / r, 0, 0, 0, 0, 1 / r, 0, 0, 0, 0, 2 * nf, 0, 0, 0, (far + near) * nf, 1]);
}

const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (a) => {
  const l = Math.hypot(...a) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

function lookAt(eye, target) {
  const z = norm(sub(eye, target));
  const x = norm(cross(Math.abs(z[1]) > 0.999 ? [0, 0, 1] : [0, 1, 0], z));
  const y = cross(z, x);
  return new Float32Array([x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, -dot(x, eye), -dot(y, eye), -dot(z, eye), 1]);
}

/** Translation × rotation (degrees, X then Y then Z as in `Rx·Ry·Rz`) × scale. */
function transform(p, rDeg, s) {
  const [a, b, c] = rDeg.map((d) => (d * Math.PI) / 180);
  const [ca, sa, cb, sb, cc, sc] = [Math.cos(a), Math.sin(a), Math.cos(b), Math.sin(b), Math.cos(c), Math.sin(c)];
  // Rx·Ry·Rz, row by row.
  const r = [
    [cb * cc, -cb * sc, sb],
    [sa * sb * cc + ca * sc, -sa * sb * sc + ca * cc, -sa * cb],
    [-ca * sb * cc + sa * sc, ca * sb * sc + sa * cc, ca * cb],
  ];
  const m = ident();
  for (let col = 0; col < 3; col++) for (let row = 0; row < 3; row++) m[col * 4 + row] = r[row][col] * s[col];
  m[12] = p[0];
  m[13] = p[1];
  m[14] = p[2];
  return m;
}

/** The inverse transpose of a model matrix's upper 3×3 (normals under non-uniform scale). */
function normalMatrix(m) {
  const [a, b, c, d, e, f, g, h, i] = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
  const A = e * i - f * h, B = -(d * i - f * g), C = d * h - e * g;
  const det = a * A + b * B + c * C || 1;
  // Inverse transpose = cofactor matrix / det, column-major.
  return new Float32Array([
    A / det, B / det, C / det,
    -(b * i - c * h) / det, (a * i - c * g) / det, -(a * h - b * g) / det,
    (b * f - c * e) / det, -(a * f - c * d) / det, (a * e - b * d) / det,
  ]);
}

// ----- geometry (unit size, centered on the origin) -----

function build(shape) {
  const pos = [], nrm = [], uv = [], idx = [];
  const vert = (p, n, t) => {
    pos.push(...p);
    nrm.push(...n);
    uv.push(...t);
    return pos.length / 3 - 1;
  };
  const quad = (a, b, c, d) => idx.push(a, b, c, a, c, d);
  // A surface of revolution or a parametric patch: f(u, v) -> [position, normal].
  const grid = (nu, nv, f) => {
    const base = pos.length / 3;
    for (let j = 0; j <= nv; j++) for (let i = 0; i <= nu; i++) vert(...f(i / nu, j / nv), [i / nu, 1 - j / nv]);
    for (let j = 0; j < nv; j++)
      for (let i = 0; i < nu; i++) {
        const k = base + j * (nu + 1) + i;
        quad(k, k + nu + 1, k + nu + 2, k + 1);
      }
  };
  const disc = (y, ny, r, n) => {
    const c = vert([0, y, 0], [0, ny, 0], [0.5, 0.5]);
    const ring = [];
    for (let i = 0; i <= n; i++) {
      const t = (i / n) * Math.PI * 2;
      ring.push(vert([Math.cos(t) * r, y, Math.sin(t) * r], [0, ny, 0], [0.5 + Math.cos(t) / 2, 0.5 + Math.sin(t) / 2]));
    }
    for (let i = 0; i < n; i++) ny > 0 ? idx.push(c, ring[i + 1], ring[i]) : idx.push(c, ring[i], ring[i + 1]);
  };
  const TAU = Math.PI * 2;
  switch (shape) {
    case "sphere":
      grid(32, 16, (u, v) => {
        const t = u * TAU, p = v * Math.PI;
        const n = [Math.cos(t) * Math.sin(p), Math.cos(p), Math.sin(t) * Math.sin(p)];
        return [n.map((x) => x * 0.5), n];
      });
      break;
    case "plane":
      grid(1, 1, (u, v) => [[u - 0.5, 0, v - 0.5], [0, 1, 0]]);
      break;
    case "cylinder":
    case "cone": {
      const top = shape === "cone" ? 0 : 0.5;
      const slope = shape === "cone" ? 0.5 : 0;
      grid(32, 1, (u, v) => {
        const t = u * TAU, r = 0.5 + (top - 0.5) * (1 - v);
        const n = norm([Math.cos(t), slope, Math.sin(t)]);
        return [[Math.cos(t) * r, v - 0.5, Math.sin(t) * r], n];
      });
      disc(-0.5, -1, 0.5, 32);
      if (top > 0) disc(0.5, 1, top, 32);
      break;
    }
    case "torus":
      grid(48, 16, (u, v) => {
        const t = u * TAU, p = v * TAU, R = 0.35, r = 0.15;
        const n = [Math.cos(t) * Math.cos(p), Math.sin(p), Math.sin(t) * Math.cos(p)];
        return [[Math.cos(t) * (R + r * Math.cos(p)), r * Math.sin(p), Math.sin(t) * (R + r * Math.cos(p))], n];
      });
      break;
    default: // box
      for (const [n, u, v] of [
        [[1, 0, 0], [0, 0, -1], [0, 1, 0]],
        [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
        [[0, 1, 0], [1, 0, 0], [0, 0, -1]],
        [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
        [[0, 0, 1], [1, 0, 0], [0, 1, 0]],
        [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
      ]) {
        const at = (su, sv) => vert([0, 1, 2].map((k) => (n[k] + su * u[k] + sv * v[k]) * 0.5), n, [(su + 1) / 2, (sv + 1) / 2]);
        quad(at(-1, -1), at(1, -1), at(1, 1), at(-1, 1));
      }
  }
  return { pos, nrm, uv, idx };
}

/** Typed arrays for WebGL, with each triangle edge once for wireframes. */
function finish({ pos, nrm, uv, idx }) {
  const seen = new Set(), edges = [];
  for (let i = 0; i < idx.length; i += 3)
    for (const [a, b] of [[idx[i], idx[i + 1]], [idx[i + 1], idx[i + 2]], [idx[i + 2], idx[i]]]) {
      const key = a < b ? a * 4294967296 + b : b * 4294967296 + a;
      if (!seen.has(key)) {
        seen.add(key);
        edges.push(a, b);
      }
    }
  const Index = pos.length / 3 > 65535 ? Uint32Array : Uint16Array;
  return { pos: new Float32Array(pos), nrm: new Float32Array(nrm), uv: new Float32Array(uv), idx: new Index(idx), edges: new Index(edges) };
}

// ----- models -----

/**
 * Centers a loaded model (its parts together) and scales it to fit a unit cube, like the
 * built-in shapes.
 */
function fit(parts) {
  const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
  for (const g of parts)
    for (let i = 0; i < g.pos.length; i += 3)
      for (let k = 0; k < 3; k++) {
        lo[k] = Math.min(lo[k], g.pos[i + k]);
        hi[k] = Math.max(hi[k], g.pos[i + k]);
      }
  const s = 1 / (Math.max(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) || 1);
  for (const g of parts)
    for (let i = 0; i < g.pos.length; i += 3) for (let k = 0; k < 3; k++) g.pos[i + k] = (g.pos[i + k] - (lo[k] + hi[k]) / 2) * s;
  return parts;
}

/** Normals of faces without them: each triangle's own (flat shading). */
function flatNormals(g) {
  const out = { ...g, pos: [], nrm: [], uv: [], idx: [] };
  for (let i = 0; i < g.idx.length; i += 3) {
    const [a, b, c] = [0, 1, 2].map((k) => g.idx[i + k]);
    const p = (j) => [g.pos[j * 3], g.pos[j * 3 + 1], g.pos[j * 3 + 2]];
    const n = norm(cross(sub(p(b), p(a)), sub(p(c), p(a))));
    for (const j of [a, b, c]) {
      out.pos.push(...p(j));
      out.nrm.push(...n);
      out.uv.push(g.uv[j * 2] ?? 0, g.uv[j * 2 + 1] ?? 0);
      out.idx.push(out.idx.length);
    }
  }
  return out;
}

/** Wavefront OBJ: `v`, `vt`, `vn` and `f` (polygons are fanned into triangles); one part. */
export function parseObj(text) {
  const v = [], vt = [], vn = [], g = { pos: [], nrm: [], uv: [], idx: [] }, ids = new Map();
  let normals = true;
  for (const line of text.split("\n")) {
    const [op, ...a] = line.trim().split(/\s+/);
    if (op === "v") v.push(a.slice(0, 3).map(Number));
    else if (op === "vt") vt.push(a.slice(0, 2).map(Number));
    else if (op === "vn") vn.push(a.slice(0, 3).map(Number));
    else if (op === "f") {
      const corner = (s) => {
        let k = ids.get(s);
        if (k === undefined) {
          const [pi, ti, ni] = s.split("/").map((x, i) => {
            const n = parseInt(x, 10), len = [v, vt, vn][i].length;
            return Number.isNaN(n) ? -1 : n < 0 ? len + n : n - 1;
          });
          g.pos.push(...(v[pi] ?? [0, 0, 0]));
          g.uv.push(...(vt[ti] ?? [0, 0]));
          if (vn[ni]) g.nrm.push(...vn[ni]);
          else normals = false;
          ids.set(s, (k = g.pos.length / 3 - 1));
        }
        return k;
      };
      const c = a.map(corner);
      for (let i = 1; i + 1 < c.length; i++) g.idx.push(c[0], c[i], c[i + 1]);
    }
  }
  return fit([normals ? g : flatNormals(g)]);
}

const quatMatrix = ([x, y, z, w], [tx, ty, tz], [sx, sy, sz]) =>
  new Float32Array([
    (1 - 2 * (y * y + z * z)) * sx, 2 * (x * y + z * w) * sx, 2 * (x * z - y * w) * sx, 0,
    2 * (x * y - z * w) * sy, (1 - 2 * (x * x + z * z)) * sy, 2 * (y * z + x * w) * sy, 0,
    2 * (x * z + y * w) * sz, 2 * (y * z - x * w) * sz, (1 - 2 * (x * x + y * y)) * sz, 0,
    tx, ty, tz, 1,
  ]);

const base64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0)).buffer;

/**
 * glTF 2.0 (`.gltf` with its buffers, or binary `.glb`): the triangles of every mesh in the
 * default scene, with node transforms applied, as one part per material. A part carries the
 * material's base color (sRGB, with alpha) and base color image: `{ uri }` (absolute) or
 * `{ bytes, mime }` for one stored in a buffer.
 */
export async function parseGltf(data, url, load) {
  let json, bin = null;
  const head = new DataView(data);
  if (data.byteLength >= 12 && head.getUint32(0, true) === 0x46546c67) {
    for (let at = 12; at + 8 <= data.byteLength; ) {
      const len = head.getUint32(at, true), type = head.getUint32(at + 4, true);
      const chunk = data.slice(at + 8, at + 8 + len);
      if (type === 0x4e4f534a) json = JSON.parse(new TextDecoder().decode(chunk));
      else if (type === 0x004e4942) bin = chunk;
      at += 8 + len;
    }
  } else json = JSON.parse(new TextDecoder().decode(data));
  const buffers = await Promise.all((json.buffers ?? []).map((b) => {
    if (b.uri === undefined) return bin;
    if (b.uri.startsWith("data:")) return base64(b.uri.slice(b.uri.indexOf(",") + 1));
    return load(new URL(b.uri, url).href);
  }));
  const SIZE = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4 };
  const TYPES = { 5120: Int8Array, 5121: Uint8Array, 5122: Int16Array, 5123: Uint16Array, 5125: Uint32Array, 5126: Float32Array };
  const MAX = { 5120: 127, 5121: 255, 5122: 32767, 5123: 65535 };
  const accessor = (i) => {
    const a = json.accessors[i], view = json.bufferViews[a.bufferView], T = TYPES[a.componentType], n = SIZE[a.type];
    const start = (view.byteOffset ?? 0) + (a.byteOffset ?? 0), stride = view.byteStride || n * T.BYTES_PER_ELEMENT;
    const src = new DataView(buffers[view.buffer]), out = new Array(a.count * n);
    const get = { 5120: "getInt8", 5121: "getUint8", 5122: "getInt16", 5123: "getUint16", 5125: "getUint32", 5126: "getFloat32" }[a.componentType];
    const scale = a.normalized ? 1 / MAX[a.componentType] : 1;
    for (let k = 0; k < a.count; k++)
      for (let c = 0; c < n; c++) out[k * n + c] = src[get](start + k * stride + c * T.BYTES_PER_ELEMENT, true) * scale;
    return out;
  };
  const parts = new Map();
  const part = (mi) => {
    let g = parts.get(mi);
    if (g) return g;
    const pbr = json.materials?.[mi]?.pbrMetallicRoughness ?? {};
    // Colors in glTF are linear; this renderer works in sRGB.
    const f = pbr.baseColorFactor;
    g = { pos: [], nrm: [], uv: [], idx: [], normals: true, color: f ? [...f.slice(0, 3).map((c) => c ** (1 / 2.2)), f[3]] : null, image: null };
    const img = json.images?.[json.textures?.[pbr.baseColorTexture?.index]?.source];
    if (img?.uri !== undefined) g.image = { uri: new URL(img.uri, url).href };
    else if (img?.bufferView !== undefined) {
      const v = json.bufferViews[img.bufferView];
      g.image = { bytes: new Uint8Array(buffers[v.buffer], v.byteOffset ?? 0, v.byteLength), mime: img.mimeType, key: `${url}#image${pbr.baseColorTexture.index}` };
    }
    parts.set(mi, g);
    return g;
  };
  const visit = (ni, parent) => {
    const node = json.nodes[ni];
    const local = node.matrix ? new Float32Array(node.matrix) : quatMatrix(node.rotation ?? [0, 0, 0, 1], node.translation ?? [0, 0, 0], node.scale ?? [1, 1, 1]);
    const m = mul(parent, local), nm = normalMatrix(m);
    for (const prim of node.mesh !== undefined ? json.meshes[node.mesh].primitives : []) {
      if ((prim.mode ?? 4) !== 4 || prim.attributes.POSITION === undefined) continue;
      const g = part(prim.material ?? -1);
      const base = g.pos.length / 3, p = accessor(prim.attributes.POSITION), count = p.length / 3;
      for (let k = 0; k < p.length; k += 3)
        for (let r = 0; r < 3; r++) g.pos.push(m[r] * p[k] + m[4 + r] * p[k + 1] + m[8 + r] * p[k + 2] + m[12 + r]);
      if (prim.attributes.NORMAL !== undefined) {
        const n = accessor(prim.attributes.NORMAL);
        for (let k = 0; k < n.length; k += 3) g.nrm.push(...norm([0, 1, 2].map((r) => nm[r] * n[k] + nm[3 + r] * n[k + 1] + nm[6 + r] * n[k + 2])));
      } else g.normals = false;
      if (prim.attributes.TEXCOORD_0 !== undefined) g.uv.push(...accessor(prim.attributes.TEXCOORD_0));
      else for (let k = 0; k < count; k++) g.uv.push(0, 0);
      const idx = prim.indices !== undefined ? accessor(prim.indices) : [...Array(count).keys()];
      for (const k of idx) g.idx.push(base + k);
    }
    for (const c of node.children ?? []) visit(c, m);
  };
  const scene = json.scenes?.[json.scene ?? 0];
  const children = new Set((json.nodes ?? []).flatMap((n) => n.children ?? []));
  for (const ni of scene ? scene.nodes : json.nodes.map((_, i) => i).filter((i) => !children.has(i))) visit(ni, ident());
  if (!parts.size) throw new Error("no triangles");
  return fit([...parts.values()].map((g) => (g.normals ? g : flatNormals(g))));
}

// ----- attributes -----

const vec3 = (s, d) => {
  const v = (s ?? "").split(" ").map(Number);
  return v.length === 3 && v.every(Number.isFinite) ? v : d;
};
const num = (s, d) => (s !== undefined && Number.isFinite(Number(s)) ? Number(s) : d);

/**
 * Creates the WebGL renderer shared by every scene of a page. `resolve` turns a model or
 * texture URL into an absolute one; `changed` is called when one has loaded (or failed).
 */
export function createScenes(doc, { resolve = (u) => u, changed = () => {} } = {}) {
  const canvas = doc.createElement("canvas");
  const gl = canvas.getContext("webgl", { antialias: true, premultipliedAlpha: true, alpha: true });
  const win = doc.defaultView;
  const colors = new Map();
  const probe = doc.createElement("canvas").getContext("2d", { willReadFrequently: true });
  /** Any CSS color as [r, g, b, a] in 0..1. */
  const color = (s, d) => {
    if (!s) return d;
    let c = colors.get(s);
    if (!c) {
      probe.clearRect(0, 0, 1, 1);
      probe.fillStyle = "#000";
      probe.fillStyle = s;
      probe.fillRect(0, 0, 1, 1);
      const p = probe.getImageData(0, 0, 1, 1).data;
      colors.set(s, (c = [p[0] / 255, p[1] / 255, p[2] / 255, p[3] / 255]));
    }
    return c;
  };
  if (!gl) return null;
  const uint = gl.getExtension("OES_element_index_uint");

  const compile = (type, src) => {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  };
  const program = (vs, fs, uniforms) => {
    const p = gl.createProgram();
    gl.attachShader(p, compile(gl.VERTEX_SHADER, vs));
    gl.attachShader(p, compile(gl.FRAGMENT_SHADER, fs));
    gl.linkProgram(p);
    const u = { p };
    for (const n of uniforms) u[n] = gl.getUniformLocation(p, n);
    u.pos = gl.getAttribLocation(p, "position");
    u.nrm = gl.getAttribLocation(p, "normal");
    u.uv = gl.getAttribLocation(p, "uv");
    return u;
  };
  const main = program(VS, FS, ["model", "viewProj", "lightViewProj", "normalMat", "color", "ambient", "eye", "pickColor", "highlight", "shininess", "opacity", "pick", "textured", "lightCount", "shadowLight", "lightPos", "lightColor", "tex", "shadowMap"]);
  const depth = program(SHADOW_VS, SHADOW_FS, ["model", "lightViewProj"]);
  gl.enable(gl.DEPTH_TEST);

  // Geometry by shape name or model URL; a model is `null` while it loads, `false` if it failed.
  const meshes = new Map();
  const upload = (d) => {
    const buf = (target, data) => {
      const b = gl.createBuffer();
      gl.bindBuffer(target, b);
      gl.bufferData(target, data, gl.STATIC_DRAW);
      return b;
    };
    const big = d.idx instanceof Uint32Array;
    if (big && !uint) throw new Error("model too large for this browser (over 65535 vertices)");
    return {
      pos: buf(gl.ARRAY_BUFFER, d.pos),
      nrm: buf(gl.ARRAY_BUFFER, d.nrm),
      uv: buf(gl.ARRAY_BUFFER, d.uv),
      idx: buf(gl.ELEMENT_ARRAY_BUFFER, d.idx),
      edges: buf(gl.ELEMENT_ARRAY_BUFFER, d.edges),
      count: d.idx.length,
      edgeCount: d.edges.length,
      type: big ? gl.UNSIGNED_INT : gl.UNSIGNED_SHORT,
    };
  };
  const fetchBytes = async (url) => {
    const r = await win.fetch(url);
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    return r.arrayBuffer();
  };
  const failed = (what, url, e) => console.warn(`[vcwl] ${what} ${url} could not be loaded: ${e.message ?? e}`);
  // A shape or model is a list of parts, each with its buffers and, for glTF, material.
  // glTF texture coordinates start at the top of the image, the others at the bottom.
  const geometry = (it) => {
    const model = it.attrs.model;
    const key = model ? resolve(model) : it.attrs.shape ?? "box";
    let g = meshes.get(key);
    if (g === undefined) {
      if (!model) meshes.set(key, (g = { flip: true, parts: [{ ...upload(finish(build(key))), color: null, image: null }] }));
      else {
        meshes.set(key, (g = null));
        fetchBytes(key)
          .then(async (data) => {
            const obj = /\.obj(\?|#|$)/i.test(key);
            const parts = obj ? parseObj(new TextDecoder().decode(data)) : await parseGltf(data, key, fetchBytes);
            meshes.set(key, { flip: obj, parts: parts.map((d) => ({ ...upload(finish(d)), color: d.color ?? null, image: d.image ?? null })) });
          })
          .catch((e) => {
            meshes.set(key, false);
            failed("model", key, e);
          })
          .finally(changed);
      }
    }
    return g;
  };

  // Textures by key: `null` while loading, `false` if it failed. `src` is a URL, or
  // `{ bytes, mime, key }` for an image stored in a model.
  const textures = new Map();
  const texture = (src, flip) => {
    const url = typeof src === "string" ? resolve(src) : null;
    const key = `${url ?? src.key} ${flip}`;
    let t = textures.get(key);
    if (t !== undefined) return t;
    textures.set(key, (t = null));
    const load = url
      ? new Promise((ok, err) => {
          const img = new win.Image();
          if (new URL(url).origin !== win.location.origin) img.crossOrigin = "anonymous";
          img.onload = () => ok(img);
          img.onerror = () => err(new Error("not an image"));
          img.src = url;
        })
      : win.createImageBitmap(new win.Blob([src.bytes], { type: src.mime }), { imageOrientation: flip ? "flipY" : "from-image" });
    load
      .then((img) => {
        const tex = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, flip && !!url);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
        gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
        // WebGL 1 repeats and mipmaps only power-of-two textures.
        const pot = (n) => (n & (n - 1)) === 0;
        if (pot(img.width) && pot(img.height)) {
          gl.generateMipmap(gl.TEXTURE_2D);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
        } else {
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        }
        gl.bindTexture(gl.TEXTURE_2D, null);
        textures.set(key, tex);
      })
      .catch((e) => {
        textures.set(key, false);
        failed("texture", url ?? src.key, e);
      })
      .finally(changed);
    return t;
  };

  const reduced = win.matchMedia?.("(prefers-reduced-motion: reduce)");
  let pickTarget = null, shadowTarget = null;
  const target = (w, h, filter) => {
    const fb = gl.createFramebuffer(), tex = gl.createTexture(), rb = gl.createRenderbuffer();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindRenderbuffer(gl.RENDERBUFFER, rb);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, w, h);
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, rb);
    // Left bound, the texture would be sampled while drawn into (a feedback loop).
    gl.bindTexture(gl.TEXTURE_2D, null);
    return { fb, tex, w, h };
  };

  /** World matrices of a scene's items at time `t` (seconds), and its camera. */
  function layout(scene, view, t) {
    const a = scene.attrs;
    const spinT = reduced?.matches ? 0 : t;
    const world = [];
    for (const it of scene.items) {
      const p = vec3(it.attrs.position, it.kind === 2 ? [3, 5, 4] : [0, 0, 0]);
      const spin = vec3(it.attrs.spin, [0, 0, 0]);
      const r = vec3(it.attrs.rotation, [0, 0, 0]).map((v, i) => v + spin[i] * spinT);
      const local = transform(p, r, vec3(it.attrs.size, [1, 1, 1]));
      world.push(it.parent >= 0 ? mul(world[it.parent], local) : local);
    }
    const target = vec3(a.target, [0, 0, 0]);
    // Orbiting turns the camera around the target.
    const off = sub(vec3(a.camera, [0, 1.5, 5]), target);
    const dist = Math.hypot(...off) || 1;
    const yaw = Math.atan2(off[0], off[2]) + view.yaw;
    const pitch = Math.max(-1.5, Math.min(1.5, Math.asin(off[1] / dist) + view.pitch));
    const eye = [target[0] + dist * Math.cos(pitch) * Math.sin(yaw), target[1] + dist * Math.sin(pitch), target[2] + dist * Math.cos(pitch) * Math.cos(yaw)];
    return { world, eye, target, fov: num(a.fov, 50) };
  }

  const bind = (prog, g) => {
    for (const [loc, b, n] of [[prog.pos, g.pos, 3], [prog.nrm, g.nrm, 3], [prog.uv, g.uv, 2]]) {
      if (loc < 0) continue;
      gl.enableVertexAttribArray(loc);
      gl.bindBuffer(gl.ARRAY_BUFFER, b);
      gl.vertexAttribPointer(loc, n, gl.FLOAT, false, 0, 0);
    }
  };

  /** Renders the depth seen from a directional light; returns its view-projection matrix. */
  function shadowPass(scene, world, center, dir) {
    // An orthographic box around everything drawn.
    let r = 1;
    scene.items.forEach((it, i) => {
      if (it.kind !== 1) return;
      const m = world[i], s = Math.hypot(m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]) / 2;
      r = Math.max(r, Math.hypot(m[12] - center[0], m[13] - center[1], m[14] - center[2]) + s);
    });
    const d = norm(dir), eye = center.map((c, k) => c + d[k] * r * 2);
    const vp = mul(ortho(r, 0.01, r * 4), lookAt(eye, center));
    shadowTarget ??= target(SHADOW_SIZE, SHADOW_SIZE, gl.NEAREST);
    gl.bindFramebuffer(gl.FRAMEBUFFER, shadowTarget.fb);
    gl.viewport(0, 0, SHADOW_SIZE, SHADOW_SIZE);
    gl.clearColor(1, 1, 1, 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.useProgram(depth.p);
    for (const l of [main.nrm, main.uv]) if (l >= 0 && l !== depth.pos) gl.disableVertexAttribArray(l);
    gl.uniformMatrix4fv(depth.lightViewProj, false, vp);
    scene.items.forEach((it, i) => {
      const m = it.kind === 1 && num(it.attrs.opacity, 1) > 0.5 && geometry(it);
      if (!m) return;
      gl.uniformMatrix4fv(depth.model, false, world[i]);
      for (const g of m.parts) {
        bind(depth, g);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, g.idx);
        gl.drawElements(gl.TRIANGLES, g.count, g.type, 0);
      }
    });
    return vp;
  }

  function render(scene, view, w, h, t, out, picking, focus) {
    const { world, eye, target: center, fov } = layout(scene, view, t);
    // Lights: without any, a soft ambient light and one from above.
    let ambient = [0, 0, 0], shadowLight = -1, shadowDir = null;
    const pos = [], col = [];
    const lights = scene.items.flatMap((it, i) => (it.kind === 2 ? [[it, [world[i][12], world[i][13], world[i][14]]]] : []));
    const defaults = [[{ attrs: { kind: "ambient", intensity: "0.35" } }, null], [{ attrs: {} }, [3, 5, 4]]];
    for (const [it, at] of lights.length ? lights : defaults) {
      const c = color(it.attrs.color, [1, 1, 1, 1]).map((v) => v * num(it.attrs.intensity, 1));
      const kind = it.attrs.kind ?? "directional";
      if (kind === "ambient") ambient = ambient.map((v, i) => v + c[i]);
      else if (pos.length < 32) {
        if (kind === "directional" && !shadowDir) {
          shadowLight = pos.length / 4;
          shadowDir = at;
        }
        pos.push(...at, kind === "point" ? 1 : 0);
        col.push(c[0], c[1], c[2]);
      }
    }
    const shadows = !picking && scene.attrs.shadows === "true" && shadowDir;
    const lightVP = shadows ? shadowPass(scene, world, center, shadowDir) : ident();

    gl.bindFramebuffer(gl.FRAMEBUFFER, out);
    gl.viewport(0, 0, w, h);
    const bg = picking ? [0, 0, 0, 1] : scene.attrs.background === "transparent" ? [0, 0, 0, 0] : color(scene.attrs.background, [0.06, 0.07, 0.08, 1]);
    gl.clearColor(bg[0] * bg[3], bg[1] * bg[3], bg[2] * bg[3], bg[3]);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    const u = main;
    gl.useProgram(u.p);
    gl.uniformMatrix4fv(u.viewProj, false, mul(perspective(fov, w / h, 0.05, 500), lookAt(eye, center)));
    gl.uniformMatrix4fv(u.lightViewProj, false, lightVP);
    gl.uniform3fv(u.eye, eye);
    gl.uniform1i(u.pick, picking ? 1 : 0);
    gl.uniform3fv(u.ambient, ambient);
    gl.uniform1i(u.lightCount, pos.length / 4);
    gl.uniform1i(u.shadowLight, shadows ? shadowLight : -1);
    if (pos.length) {
      gl.uniform4fv(u.lightPos, pos);
      gl.uniform3fv(u.lightColor, col);
    }
    gl.uniform1i(u.tex, 0);
    gl.uniform1i(u.shadowMap, 1);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, shadows ? shadowTarget.tex : null);
    gl.activeTexture(gl.TEXTURE0);

    // Opaque meshes first, then translucent ones from far to near without writing depth.
    const draws = [];
    scene.items.forEach((it, i) => {
      const model = it.kind === 1 && geometry(it);
      if (!model) return;
      const m = world[i], far = Math.hypot(m[12] - eye[0], m[13] - eye[1], m[14] - eye[2]);
      for (const g of model.parts) {
        const opacity = picking ? 1 : Math.max(0, Math.min(1, num(it.attrs.opacity, 1) * (g.color?.[3] ?? 1)));
        if (opacity > 0) draws.push({ it, i, g, flip: model.flip, opacity, far });
      }
    });
    const solid = draws.filter((d) => d.opacity >= 1);
    const glass = draws.filter((d) => d.opacity < 1).sort((a, b) => b.far - a.far);
    for (const { it, i, g, flip, opacity } of [...solid, ...glass]) {
      const blend = opacity < 1;
      if (blend) {
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      }
      gl.depthMask(!blend);
      bind(u, g);
      gl.uniformMatrix4fv(u.model, false, world[i]);
      gl.uniformMatrix3fv(u.normalMat, false, normalMatrix(world[i]));
      // The mesh's texture replaces the material's; the mesh's color multiplies it.
      const image = it.attrs.texture ?? g.image;
      const tex = image ? texture(image.uri ?? image, flip) : null;
      gl.uniform1i(u.textured, tex ? 1 : 0);
      if (tex) gl.bindTexture(gl.TEXTURE_2D, tex);
      const plain = image || g.color ? [1, 1, 1, 1] : [0.82, 0.42, 0.28, 1];
      const c = color(it.attrs.color, plain).map((v, k) => v * (g.color?.[k] ?? 1));
      gl.uniform3f(u.color, c[0], c[1], c[2]);
      gl.uniform1f(u.shininess, num(it.attrs.shininess, 32));
      gl.uniform1f(u.opacity, opacity);
      gl.uniform3fv(u.highlight, it.id === focus && !picking ? [0.3, 0.55, 1] : [0, 0, 0]);
      // Pick colors are item numbers + 1; 0 (the background) for meshes without a handler,
      // which still hide what is behind them.
      const k = it.clickable ? i + 1 : 0;
      gl.uniform3f(u.pickColor, (k & 255) / 255, ((k >> 8) & 255) / 255, ((k >> 16) & 255) / 255);
      const wire = it.attrs.wireframe === "true" && !picking;
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, wire ? g.edges : g.idx);
      gl.drawElements(wire ? gl.LINES : gl.TRIANGLES, wire ? g.edgeCount : g.count, g.type, 0);
      if (blend) gl.disable(gl.BLEND);
    }
    gl.depthMask(true);
  }

  return {
    /** Whether a scene moves by itself (spinning items). */
    animated(scene) {
      return !reduced?.matches && scene.items.some((it) => it.attrs.spin && vec3(it.attrs.spin, [0, 0, 0]).some((v) => v !== 0));
    },
    /** Draws `scene` at `w`×`h` device pixels; returns the canvas holding the image. */
    draw(scene, view, w, h, t, focus = -1) {
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
      render(scene, view, w, h, t, null, false, focus);
      return canvas;
    },
    /** The node id of the clickable mesh at device pixel (x, y), or -1. */
    pick(scene, view, w, h, t, x, y) {
      // An offscreen target without antialiasing, so pick colors are exact.
      if (!pickTarget || pickTarget.w !== w || pickTarget.h !== h) pickTarget = target(w, h, gl.NEAREST);
      render(scene, view, w, h, t, pickTarget.fb, true, -1);
      const px = new Uint8Array(4);
      gl.bindFramebuffer(gl.FRAMEBUFFER, pickTarget.fb);
      gl.readPixels(Math.round(x), Math.round(h - 1 - y), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      const k = px[0] + (px[1] << 8) + (px[2] << 16);
      return k > 0 && scene.items[k - 1] ? scene.items[k - 1].id : -1;
    },
  };
}
