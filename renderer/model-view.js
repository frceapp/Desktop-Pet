// Shows a character from a glTF model. The model needs no rig or animation clips:
// movement is done by turning, hopping and squashing the whole figure, the arms, legs
// and ears are found by their shape and moved on their own, and facial expressions are
// made by repainting the face texture (see `expressions` in character.json).

import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';

const FACE_TEXTURE_SIZE = 1024; // expression textures are downscaled to this; plenty for a pet
const FRAME = 1.3; // camera frame height relative to the model, leaving room to hop and lean
const TEAR_COUNT = 4;
const FIDGETS = ['wave', 'cheer', 'look', 'wiggle']; // things it does by itself while idle
const FIDGET_SECONDS = 1.4;
// How far an arm may lift from hanging, in radians: a little above level. These characters'
// heads are far wider than their shoulders, so any higher and the arm disappears into the head.
const ARM_UP = 1.15;

// Returns the face texture's pixels with an expression's edits applied, as
// { data, width, height }. Coordinates in the ops are pixels of the original image.
function paint(image, ops, ink) {
  const canvas = document.createElement('canvas');
  canvas.width = image.width;
  canvas.height = image.height;
  const g = canvas.getContext('2d', { willReadFrequently: true });
  g.drawImage(image, 0, 0);
  for (const op of ops) {
    if (op.clear) {
      g.clearRect(...op.clear);
    } else if (op.copy) {
      const [sx, sy, w, h, dx, dy, dw = w, dh = h] = op.copy;
      g.drawImage(image, sx, sy, w, h, dx, dy, dw, dh);
    } else if (op.stroke) {
      const [width, ...points] = op.stroke;
      g.lineWidth = width;
      g.lineCap = g.lineJoin = 'round';
      g.strokeStyle = ink;
      g.beginPath();
      points.forEach(([x, y], i) => (i ? g.lineTo(x, y) : g.moveTo(x, y)));
      g.stroke();
    }
  }

  // A canvas forgets the colour of see-through pixels (they read back black), while the
  // image file keeps them white. That colour bleeds into the soft edge of every line once
  // the texture is shrunk, so put the white back or lines come out visibly thicker than on
  // the untouched face.
  let { width, height } = canvas;
  let data = g.getImageData(0, 0, width, height).data;
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] === 0) data[i] = data[i + 1] = data[i + 2] = 255;
  }
  // Shrink by plain averaging, exactly as the graphics card builds its smaller copies.
  while (width > FACE_TEXTURE_SIZE && width % 2 === 0 && height % 2 === 0) {
    const half = new Uint8ClampedArray((width / 2) * (height / 2) * 4);
    for (let y = 0; y < height; y += 2) {
      for (let x = 0; x < width; x += 2) {
        const a = (y * width + x) * 4;
        const b = a + width * 4;
        const o = ((y / 2) * (width / 2) + x / 2) * 4;
        for (let c = 0; c < 4; c++) half[o + c] = (data[a + c] + data[a + 4 + c] + data[b + c] + data[b + 4 + c] + 2) >> 2;
      }
    }
    data = half;
    width /= 2;
    height /= 2;
  }
  return { data, width, height };
}

// Splits a mesh into its separate shapes (on a simple model the ears, tail, arms and legs
// are each their own blob stuck into the body). Returns { indices, solid } per shape;
// `solid` is false for open surfaces, like eyes and cheeks laid flat on the face.
function pieces(geometry) {
  const position = geometry.attributes.position;
  const index = geometry.index;
  if (!index) return [];
  const parent = Array.from({ length: position.count }, (_, i) => i);
  const find = (i) => {
    while (parent[i] !== i) i = parent[i] = parent[parent[i]];
    return i;
  };
  const join = (a, b) => (parent[find(a)] = find(b));
  // vertices are duplicated along seams, so weld the ones sharing a position
  const at = new Map();
  const welded = [];
  for (let i = 0; i < position.count; i++) {
    const key = [position.getX(i), position.getY(i), position.getZ(i)].map((n) => Math.round(n * 1e4)).join();
    if (!at.has(key)) at.set(key, i);
    welded[i] = at.get(key);
    join(i, welded[i]);
  }
  // a solid has every edge shared by two triangles; an edge used once is a rim
  const uses = new Map();
  for (let i = 0; i < index.count; i += 3) {
    const corners = [index.getX(i), index.getX(i + 1), index.getX(i + 2)];
    corners.forEach((corner, n) => {
      const next = corners[(n + 1) % 3];
      join(corner, next);
      const a = welded[corner];
      const b = welded[next];
      const edge = a < b ? `${a} ${b}` : `${b} ${a}`;
      uses.set(edge, (uses.get(edge) ?? 0) + 1);
    });
  }
  const open = new Set();
  for (const [edge, count] of uses) {
    if (count === 1) open.add(find(Number(edge.split(' ')[0])));
  }
  const groups = new Map();
  for (let i = 0; i < position.count; i++) {
    const root = find(i);
    if (!groups.has(root)) groups.set(root, { indices: [], solid: !open.has(root) });
    groups.get(root).indices.push(i);
  }
  return [...groups.values()];
}

// Picks the limbs out of a figure's pieces by where they are: legs are the short pieces
// standing on the floor, arms the small ones out at the sides around the middle, ears the
// ones off to either side that reach the very top.
// Each is remembered in the figure's own space (x sideways, y up, z toward the viewer)
// with the joint it turns about. `belly` is how far forward the body reaches at leg height.
function findLimbs(figure, size) {
  const legs = [];
  const arms = [];
  const ears = [];
  let belly = 0;
  const point = new THREE.Vector3();
  figure.traverse((mesh) => {
    if (!mesh.isMesh) return;
    const position = mesh.geometry.attributes.position;
    for (const { indices, solid } of pieces(mesh.geometry)) {
      const rest = new Float32Array(indices.length * 3);
      const box = new THREE.Box3();
      indices.forEach((index, i) => {
        point.fromBufferAttribute(position, index).applyMatrix4(mesh.matrixWorld).toArray(rest, i * 3);
        box.expandByPoint(point);
      });
      const tall = box.max.y - box.min.y;
      const wide = box.max.x - box.min.x;
      const middle = box.getCenter(new THREE.Vector3());
      const side = Math.sign(middle.x);
      const limb = { position, indices, rest, tall, side, toLocal: mesh.matrixWorld.clone().invert() };

      if (box.min.y > size.y * 0.6 && box.max.y > size.y * 0.9 && Math.abs(middle.x) > size.x * 0.08) {
        // An ear's lining can be a flat patch, so ears need not be solid. It hinges at its root.
        limb.joint = new THREE.Vector3(middle.x, box.min.y + tall * 0.1, middle.z);
        ears.push(limb);
      } else if (!solid) {
        continue;
      } else if (box.min.y <= size.y * 0.03 && box.max.y <= size.y * 0.25) {
        limb.joint = new THREE.Vector3(middle.x, box.min.y + tall * 0.65, middle.z); // hip
        legs.push(limb);
      } else if (
        Math.abs(middle.x) > size.x * 0.2 &&
        middle.y > size.y * 0.15 &&
        middle.y < size.y * 0.6 &&
        tall < size.y * 0.4
      ) {
        // shoulder: the upper end nearest the body
        limb.joint = new THREE.Vector3(middle.x - side * wide * 0.35, box.max.y - tall * 0.2, middle.z);
        arms.push(limb);
      } else {
        for (let i = 0; i < rest.length; i += 3) {
          if (rest[i + 1] < size.y * 0.2) belly = Math.max(belly, rest[i + 2]);
        }
        continue;
      }
      mesh.frustumCulled = false; // its bounds no longer match once limbs move
    }
  });
  // A limb usually exists twice, as the body mesh and as its slightly larger outline shell.
  // Both must turn about the very same point or the outline drifts off the limb.
  for (const limbs of [legs, arms, ears]) {
    for (const side of [-1, 1]) {
      const same = limbs.filter((limb) => limb.side === side);
      const joint = same.reduce((sum, limb) => sum.add(limb.joint), new THREE.Vector3()).divideScalar(same.length || 1);
      for (const limb of same) limb.joint = joint;
    }
  }
  return { legs, arms, ears, belly };
}

export async function createView(host, character, cfg) {
  const opts = cfg.model;
  const gltf = await new GLTFLoader().loadAsync(new URL(opts.file, character.base).href);
  const figure = opts.node ? gltf.scene.getObjectByName(opts.node) : gltf.scene;
  if (!figure) throw new Error(`Model has no node named "${opts.node}"`);

  // A file can hold several characters; show only the requested one.
  for (let o = figure; o.parent; o = o.parent) {
    for (const sibling of o.parent.children) sibling.visible = sibling === o;
  }

  // Stand the figure on the origin so it turns around its own vertical axis.
  gltf.scene.updateMatrixWorld(true);
  const box = new THREE.Box3().setFromObject(figure);
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  gltf.scene.position.set(-center.x, -box.min.y, -center.z);
  gltf.scene.updateMatrixWorld(true);
  const { legs, arms, ears, belly } = findLimbs(figure, size);
  const legHeight = legs[0]?.tall ?? 0;
  const pivot = new THREE.Group();
  pivot.rotation.order = 'YXZ';
  pivot.add(gltf.scene);
  const scene = new THREE.Scene();
  scene.add(pivot);

  const span = size.y * FRAME;
  const camera = new THREE.OrthographicCamera(-span / 2, span / 2, span * 0.97, -span * 0.03, 0.1, 100);
  camera.position.z = 10;

  const canvas = document.createElement('canvas');
  host.replaceChildren(canvas);
  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true });
  renderer.setPixelRatio(Math.max(2, devicePixelRatio));
  renderer.setSize(cfg.size, cfg.size, false);

  // --- expressions: swap the face material's texture for a repainted copy
  const faces = [];
  figure.traverse((o) => {
    if (o.isMesh && o.material.map) faces.push(o.material);
  });
  const original = faces[0]?.map;
  // The everyday face is the model's own unless the character repaints that too.
  const textures = opts.expressions?.neutral ? {} : { neutral: original };
  // An expression may name another one to start from, e.g. ["neutral", { ... }].
  const edits = (name) => opts.expressions[name].flatMap((op) => (typeof op === 'string' ? edits(op) : op));
  function show(name) {
    if (!original) return;
    if (!textures[name]) {
      const { data, width, height } = paint(original.image, edits(name), opts.ink ?? '#391a1c');
      const texture = new THREE.DataTexture(data, width, height, THREE.RGBAFormat);
      for (const key of ['colorSpace', 'wrapS', 'wrapT', 'minFilter', 'magFilter', 'generateMipmaps', 'anisotropy']) {
        texture[key] = original[key];
      }
      texture.needsUpdate = true;
      textures[name] = texture;
    }
    for (const face of faces) face.map = textures[name];
  }
  show('neutral');

  // --- tears: drops that run down from under each eye while crying
  const tears = [];
  if (opts.tears) {
    const material = new THREE.MeshBasicMaterial({ color: opts.tears.color ?? '#8ed6ff' });
    const geometry = new THREE.SphereGeometry(size.y * 0.035, 12, 8);
    for (let i = 0; i < TEAR_COUNT; i++) {
      const drop = new THREE.Mesh(geometry, material);
      drop.visible = false;
      pivot.add(drop);
      tears.push(drop);
    }
  }

  // --- limbs: turn each about its joint, then nudge it
  const turnLimb = new THREE.Quaternion();
  const euler = new THREE.Euler();
  const nudge = new THREE.Vector3();
  const point = new THREE.Vector3();
  function place(limb) {
    limb.indices.forEach((index, i) => {
      point.fromArray(limb.rest, i * 3).sub(limb.joint).applyQuaternion(turnLimb).add(limb.joint).add(nudge).applyMatrix4(limb.toLocal);
      limb.position.setXYZ(index, point.x, point.y, point.z);
    });
    limb.position.needsUpdate = true;
  }
  // `step` is -1..1 through a walking stride, `sit` 0..1 from standing to sitting,
  // `raise` how far each arm is lifted out to the side and `lean` how far each ear tips
  // [outward, forward], all in radians and keyed by side (-1, 1).
  function poseLimbs(step, sit, raise, lean) {
    for (const leg of legs) {
      const stride = step * leg.side;
      // Swing forward and back when walking. Sitting, they stick straight out from the front
      // of the belly, a little apart, so the soles show.
      turnLimb.setFromEuler(euler.set(-stride * 0.7 - sit * 1.35, sit * 0.3 * leg.side, 0, 'YXZ'));
      nudge.set(0, Math.max(0, stride) * leg.tall * 0.3, sit * (belly - leg.joint.z + leg.tall * 0.35));
      place(leg);
    }
    for (const arm of arms) {
      // each arm swings against the leg on its own side
      const lift = Math.min(raise[arm.side], ARM_UP);
      turnLimb.setFromEuler(euler.set(step * arm.side * 0.5, 0, lift * arm.side, 'XYZ'));
      nudge.set(0, 0, 0);
      place(arm);
    }
    for (const ear of ears) {
      const [out, forward] = lean[ear.side];
      turnLimb.setFromEuler(euler.set(forward, 0, -out * ear.side, 'XYZ'));
      place(ear);
    }
  }

  let state = 'idle';
  let expression = 'neutral';
  let facing = 1;
  let sitting = false;
  let sit = 0;
  let yaw = 0;
  let clock = 0;
  let since = 0; // seconds in the current state
  let blinkIn = 3;
  let fidget = null; // { kind, time, side } while one is playing
  let fidgetIn = 1;
  let flick = null; // { time, side }: one ear giving a quick double twitch
  let flickIn = 2;

  function setState(next) {
    if (next === state) return;
    state = next;
    since = 0;
    fidget = null;
    fidgetIn = 0.4 + Math.random() * 1.2;
    const has = (name) => Boolean(opts.expressions?.[name]);
    // borrowed when it has no face of its own for this state
    const stand = { drag: 'talk', fall: 'talk', chase: 'talk', reach: 'talk', greet: 'talk', chat: 'talk', dance: 'talk', special: 'annoyed', sleep: 'blink' }[next];
    expression = has(next) ? next : has(stand) ? stand : 'neutral';
    show(expression);
  }

  function update(dt) {
    clock += dt;
    since += dt;
    let turn = 0;
    let hop = 0;
    let roll = 0;
    let pitch = 0;
    let shift = 0;
    let stretch = 1;
    let step = 0;
    const raise = { [-1]: 0, 1: 0 };
    // ears always drift a little, each on its own beat
    const lean = { [-1]: [Math.sin(clock * 1.7) * 0.05, 0], 1: [Math.sin(clock * 1.7 + 2) * 0.05, 0] };
    const bothEars = (out, forward) => {
      for (const side of [-1, 1]) {
        lean[side][0] += out;
        lean[side][1] += forward;
      }
    };

    switch (state) {
      case 'walk':
        turn = facing * 0.6;
        step = Math.sin(clock * 9);
        hop = Math.abs(step) * 0.03;
        roll = step * 0.06;
        raise[-1] = raise[1] = 0.15;
        // they flop back and forth a beat behind each step
        bothEars(Math.sin(clock * 9 - 1.2) * 0.07, -0.12 + Math.sin(clock * 18 - 1.2) * 0.2);
        break;
      case 'talk':
        hop = Math.abs(Math.sin(since * 11)) * 0.07 * Math.exp(-since * 3);
        stretch = 1 + Math.sin(since * 16) * 0.03;
        raise[-1] = raise[1] = 0.55 + Math.sin(since * 14) * 0.25;
        bothEars(0.08 + Math.sin(since * 14) * 0.08, 0);
        break;
      case 'annoyed':
        // shaking its head "no" and flailing
        turn = Math.sin(since * 20) * (0.5 - sit * 0.3);
        yaw = turn;
        stretch = 1 + Math.sin(since * 40) * 0.02;
        raise[-1] = 0.45 + Math.sin(since * 26) * 0.45;
        raise[1] = 0.45 - Math.sin(since * 26) * 0.45;
        // flattened out sideways and shaking
        bothEars(0.5 + Math.sin(since * 32) * 0.1, 0);
        break;
      case 'special':
        // arms thrown up, floating, ears bolt upright and buzzing
        turn = facing * 0.3;
        hop = 0.05 + Math.sin(since * 5) * 0.02;
        stretch = 1 + Math.sin(since * 30) * 0.015;
        raise[-1] = raise[1] = 1.05 + Math.sin(since * 22) * 0.1;
        bothEars(-0.06 + Math.sin(since * 48) * 0.07, Math.sin(since * 37) * 0.08);
        break;
      case 'cry':
        pitch = 0.1;
        shift = Math.sin(since * 55) * 0.008;
        stretch = 1 + Math.sin(since * 11) * 0.025;
        raise[-1] = raise[1] = 0.6 + Math.sin(since * 30) * 0.08;
        bothEars(0.7, 0.25); // drooping
        break;
      case 'chase':
        // running after the mouse pointer with both arms out
        turn = facing * 0.6;
        step = Math.sin(clock * 11);
        hop = Math.abs(step) * 0.04;
        roll = step * 0.07;
        raise[-1] = raise[1] = 0.95 + Math.sin(clock * 11) * 0.15;
        bothEars(0.05, -0.3);
        break;
      case 'reach':
        // right under it: stretching up, grabbing with one hand then the other, little jumps
        turn = facing * 0.25;
        hop = Math.abs(Math.sin(since * 7)) * 0.1;
        stretch = 1.04 + Math.sin(since * 14) * 0.02;
        raise[-1] = 0.85 + Math.sin(since * 16) * 0.3;
        raise[1] = 0.85 - Math.sin(since * 16) * 0.3;
        bothEars(-0.05 + Math.sin(since * 14) * 0.05, 0);
        break;
      // --- with another pet (it is already turned to face them)
      case 'greet':
        // a little hop, then waving with the hand nearest them
        turn = facing * 0.5;
        hop = Math.abs(Math.sin(since * 9)) * 0.08 * Math.exp(-since * 2);
        raise[facing] = 0.85 + Math.sin(since * 15) * 0.3;
        raise[-facing] = 0.2;
        bothEars(0.1 + Math.sin(since * 15) * 0.06, 0);
        break;
      case 'chat':
        // nodding along and gesturing while they talk
        turn = facing * 0.5;
        pitch = Math.max(0, Math.sin(since * 3.4)) * 0.07;
        stretch = 1 + Math.sin(since * 12) * 0.02;
        raise[-1] = 0.4 + Math.sin(since * 5) * 0.3;
        raise[1] = 0.4 - Math.sin(since * 5) * 0.3;
        break;
      case 'dance':
        // bouncing side to side together, arms going up in turn
        turn = facing * 0.3 + Math.sin(since * 4.5) * 0.25;
        hop = Math.abs(Math.sin(since * 9)) * 0.07;
        roll = Math.sin(since * 9) * 0.1;
        raise[-1] = 0.6 + Math.sin(since * 9) * 0.5;
        raise[1] = 0.6 - Math.sin(since * 9) * 0.5;
        bothEars(Math.sin(since * 9) * 0.15, Math.sin(since * 18) * 0.12);
        break;
      case 'comfort':
        // leaning in against someone who is crying, patting them with the near hand
        turn = facing * 0.55;
        roll = -facing * 0.09;
        raise[facing] = 0.6 + Math.abs(Math.sin(since * 6)) * 0.5;
        bothEars(0.25, 0.1);
        break;
      case 'sleep':
        // dozing: slow deep breaths, nodding forward, ears gone slack
        pitch = 0.12 + Math.sin(clock * 0.9) * 0.05;
        roll = Math.sin(clock * 0.45) * 0.04;
        stretch = 1 + Math.sin(clock * 1.5) * 0.03;
        bothEars(0.3, 0.2);
        break;
      case 'drag':
        roll = Math.sin(since * 7) * 0.18;
        stretch = 1.06;
        raise[-1] = 0.85 + Math.sin(since * 10) * 0.3;
        raise[1] = 0.85 - Math.sin(since * 10) * 0.3;
        lean[-1][0] -= Math.sin(since * 7 - 1) * 0.3;
        lean[1][0] += Math.sin(since * 7 - 1) * 0.3;
        break;
      case 'fall':
        stretch = 1.1;
        raise[-1] = raise[1] = ARM_UP;
        bothEars(0.15, -0.5); // streaming up behind it
        break;
      default: {
        turn = Math.sin(clock * 0.6) * 0.12;
        stretch = 1 + Math.sin(clock * 2.6) * 0.015;
        raise[-1] = raise[1] = Math.sin(clock * 2.6) * 0.06;

        fidgetIn -= dt;
        if (!fidget && fidgetIn <= 0) {
          fidget = { kind: FIDGETS[Math.floor(Math.random() * FIDGETS.length)], time: 0, side: Math.random() < 0.5 ? -1 : 1 };
        }
        if (!fidget) break;
        fidget.time += dt;
        const t = fidget.time;
        const part = t / FIDGET_SECONDS; // 0..1 through the fidget
        const ease = Math.min(1, Math.sin(Math.PI * part) * 3); // quick in, hold, quick out
        if (fidget.kind === 'wave') {
          raise[fidget.side] = ease * (0.85 + Math.sin(t * 16) * 0.3);
        } else if (fidget.kind === 'cheer') {
          raise[-1] = raise[1] = ease * ARM_UP;
          hop = Math.abs(Math.sin(t * 10)) * 0.06 * ease;
        } else if (fidget.kind === 'look') {
          turn = Math.sin(Math.PI * 2 * part) * 0.5;
        } else {
          roll = Math.sin(t * 9) * 0.09 * ease;
          raise[-1] = ease * (0.5 + Math.sin(t * 9) * 0.4);
          raise[1] = ease * (0.5 - Math.sin(t * 9) * 0.4);
        }
        if (part >= 1) {
          fidget = null;
          fidgetIn = 0.8 + Math.random() * 2;
        }
      }
    }

    // every so often one ear gives a quick double twitch
    flickIn -= dt;
    if (!flick && flickIn <= 0) flick = { time: 0, side: Math.random() < 0.5 ? -1 : 1 };
    if (flick) {
      flick.time += dt;
      lean[flick.side][0] += Math.abs(Math.sin(flick.time * 22)) * 0.45 * Math.sin((Math.PI * flick.time) / 0.45);
      if (flick.time >= 0.45) {
        flick = null;
        flickIn = 1.2 + Math.random() * 3;
      }
    }

    const upright = ['walk', 'chase', 'drag', 'fall'].includes(state);
    const seated = state === 'sleep' || (sitting && !upright);
    sit += ((seated ? 1 : 0) - sit) * Math.min(1, dt * 8);
    if (Math.abs(sit - (seated ? 1 : 0)) < 0.002) sit = seated ? 1 : 0;
    poseLimbs(step, sit, raise, lean);

    yaw += (turn - yaw) * Math.min(1, dt * 10);
    pivot.rotation.set(pitch, yaw + THREE.MathUtils.degToRad(opts.yaw ?? 0), roll);
    // sitting lowers the body until it rests on the floor
    pivot.position.set(shift * size.y, hop * size.y * (1 - sit) - sit * legHeight * 0.45, 0);
    pivot.scale.set(1 / Math.sqrt(stretch), stretch, 1 / Math.sqrt(stretch));

    if (expression === 'neutral' && opts.expressions?.blink) {
      blinkIn -= dt;
      if (blinkIn < -0.13) blinkIn = 2.5 + Math.random() * 3;
      show(blinkIn < 0 ? 'blink' : 'neutral');
    } else if (expression === 'talk' && (state === 'talk' || state === 'greet' || state === 'chat')) {
      // the mouth opens and closes while it speaks
      show(Math.sin(since * 20) > -0.3 ? 'talk' : 'neutral');
    }

    tears.forEach((drop, i) => {
      drop.visible = state === 'cry';
      if (!drop.visible) return;
      const side = i % 2 ? 1 : -1;
      const fall = (since * 1.1 + i / TEAR_COUNT) % 1;
      drop.position.set(
        side * size.x * (opts.tears.x + fall * 0.05),
        size.y * (opts.tears.y - fall * 0.45),
        size.z / 2,
      );
      drop.scale.set(1, 1.5, 1).multiplyScalar(1 - fall * 0.5);
    });

    renderer.render(scene, camera);
  }

  return {
    // how much of its square box the figure fills, sideways and upward
    width: size.x / span,
    height: size.y / span + 0.03,
    setState,
    setFacing: (dir) => (facing = dir),
    setSitting: (value) => (sitting = value),
    resize: (px) => renderer.setSize(px, px, false),
    update,
    dispose() {
      for (const texture of Object.values(textures)) texture?.dispose();
      original?.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      canvas.remove();
    },
  };
}
