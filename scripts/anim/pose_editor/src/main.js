import * as THREE from 'three';
import { OrbitControls } from 'three-addons/controls/OrbitControls.js';
import { TransformControls } from 'three-addons/controls/TransformControls.js';
import { GLTFLoader } from 'three-addons/loaders/GLTFLoader.js';

const params = new URLSearchParams(location.search);
const CLIP = params.get('clip') ?? 'kick_strike';

const view = document.getElementById('view');
const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(devicePixelRatio);
view.appendChild(renderer.domElement);
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x22262b);
const camera = new THREE.PerspectiveCamera(45, 1, 0.05, 100);
const orbit = new OrbitControls(camera, renderer.domElement);

scene.add(new THREE.HemisphereLight(0xdfe8ef, 0x3a3f45, 1.1));
const sun = new THREE.DirectionalLight(0xffffff, 1.6);
sun.position.set(3, 6, 4);
scene.add(sun);

let GROUND_Y = -0.87;
const grid = new THREE.GridHelper(8, 32, 0x55606a, 0x3a4149);
scene.add(grid);

function setCam(name) {
  const t = new THREE.Vector3(0, 0.05, 0);
  const pos = { front: [0, 0.2, 4.4], side: [4.4, 0.2, 0], back: [0, 0.2, -4.4],
                tq: [3.1, 0.9, 3.1] }[name];
  camera.position.set(...pos);
  orbit.target.copy(t);
  orbit.update();
}
setCam('tq');

function resize() {
  const w = view.clientWidth, h = view.clientHeight;
  renderer.setSize(w, h);
  camera.aspect = w / h;
  camera.updateProjectionMatrix();
}
addEventListener('resize', resize);

// ---------- state ----------
let clip = null;          // the editable clip json
let bones = {};           // name -> THREE.Bone
let restQ = {};           // name -> rest local quaternion
let charRoot = null;
let curFrame = 1;
let selected = null;
let playing = false;
let dirty = false;
const status = (m) => { document.getElementById('status').textContent = m; };

// clip picker: switch animations via a full reload so state stays simple;
// guard unsaved edits (they live only in this tab until 💾)
fetch('/clips').then((r) => r.json()).then(({ clips }) => {
  const sel = document.getElementById('clipsel');
  clips.forEach((n) => {
    const el = document.createElement('option');
    el.value = n; el.textContent = n; sel.appendChild(el);
  });
  sel.value = CLIP;
  sel.onchange = () => {
    if (dirty && !confirm('Unsaved changes will be lost — switch anyway?')) {
      sel.value = CLIP;
      return;
    }
    location.href = `?clip=${sel.value}`;
  };
});
addEventListener('beforeunload', (e) => { if (dirty) e.preventDefault(); });

const loader = new GLTFLoader();
loader.load('data/character.glb', (g) => {
  charRoot = g.scene;
  scene.add(charRoot);
  const rawBones = {};
  charRoot.traverse((o) => {
    if (o.isBone) rawBones[o.name] = o;
    if (o.isSkinnedMesh) { o.frustumCulled = false; }
  });
  const bb = new THREE.Box3().setFromObject(charRoot);
  GROUND_Y = bb.min.y;
  grid.position.y = GROUND_Y + 0.001;
  fetch(`data/${CLIP}.clip.json`).then((r) => r.json()).then((c) => {
    clip = c;
    // Blender's glTF export strips dots from bone names ("foot.R" -> "footR");
    // key everything by the clip's canonical names
    for (const n of c.bone_order) {
      const gb = rawBones[n] ?? rawBones[n.replace(/\./g, '')];
      if (gb) { bones[n] = gb; restQ[n] = gb.quaternion.clone(); }
    }
    const sel = document.getElementById('bonesel');
    Object.keys(bones).sort().forEach((n) => {
      const el = document.createElement('option');
      el.value = n; el.textContent = n; sel.appendChild(el);
    });
    buildPickers();
    document.getElementById('clipname').textContent =
      `${c.name} — ${c.keys.length} keys @ ${c.fps} fps`;
    const fr = document.getElementById('frame');
    fr.min = c.frame_start; fr.max = c.frame_end;
    buildKeyButtons();
    setFrame(c.frame_start);
    status('loaded');
  });
});

// ---------- pose application ----------
function keyAt(f) { return clip.keys.find((k) => Math.abs(k.frame - f) < 1e-6); }

function poseAt(f) {
  const ks = clip.keys;
  let a = ks[0], b = ks[ks.length - 1];
  for (let i = 0; i < ks.length - 1; i++) {
    if (f >= ks[i].frame && f <= ks[i + 1].frame) { a = ks[i]; b = ks[i + 1]; break; }
  }
  const u = a === b ? 0 : (f - a.frame) / (b.frame - a.frame);
  const names = new Set([...Object.keys(a.bones), ...Object.keys(b.bones)]);
  const out = {};
  const qa = new THREE.Quaternion(), qb = new THREE.Quaternion();
  for (const n of names) {
    const A = a.bones[n], B = b.bones[n];
    qa.set(A ? A[1] : 0, A ? A[2] : 0, A ? A[3] : 0, A ? A[0] : 1);
    qb.set(B ? B[1] : 0, B ? B[2] : 0, B ? B[3] : 0, B ? B[0] : 1);
    out[n] = qa.slerp(qb, u).clone();
  }
  out.__rootz = (a.root_z ?? 0) * (1 - u) + (b.root_z ?? 0) * u;
  return out;
}

function applyFrame(f) {
  if (!clip || !charRoot) return;
  const pose = poseAt(f);
  for (const [n, b] of Object.entries(bones)) {
    const p = pose[n];
    b.quaternion.copy(restQ[n]);
    if (p) b.quaternion.multiply(p);
  }
  charRoot.position.y = pose.__rootz ?? 0;
  updatePickers();
}

function setFrame(f) {
  curFrame = f;
  document.getElementById('frame').value = f;
  const k = keyAt(f);
  document.getElementById('framelabel').textContent =
    `${f}${k ? ' (key)' : ''}`;
  document.getElementById('mode').textContent =
    k ? 'ON KEY — editing enabled' : 'between keys — jump to a key to edit';
  document.querySelectorAll('#keys button').forEach((b) =>
    b.classList.toggle('on', Number(b.dataset.f) === f));
  gizmo.enabled = !!k && !!selected;
  gizmo.visible = gizmo.enabled;
  applyFrame(f);
  refreshBoneInfo();
}

function buildKeyButtons() {
  const holder = document.getElementById('keys');
  holder.innerHTML = '';
  for (const k of clip.keys) {
    const b = document.createElement('button');
    b.className = 'key'; b.textContent = k.frame; b.dataset.f = k.frame;
    b.onclick = () => setFrame(k.frame);
    holder.appendChild(b);
  }
}

// ---------- picking ----------
const pickers = new THREE.Group();
scene.add(pickers);
const pickMat = new THREE.MeshBasicMaterial({ color: 0xffa040, depthTest: false,
  transparent: true, opacity: 0.85 });
const pickSel = new THREE.MeshBasicMaterial({ color: 0x40ff80, depthTest: false });
function buildPickers() {
  const g = new THREE.SphereGeometry(0.016, 10, 8);
  for (const n of Object.keys(bones)) {
    if (n.includes('_tip') || n.match(/(thumb|index|middle|ring|pinky)_/)) continue;
    const m = new THREE.Mesh(g, pickMat);
    m.userData.bone = n; m.renderOrder = 10;
    pickers.add(m);
  }
}
function updatePickers() {
  const v = new THREE.Vector3();
  for (const m of pickers.children) {
    bones[m.userData.bone].getWorldPosition(v);
    m.position.copy(v);
    m.material = m.userData.bone === selected ? pickSel : pickMat;
  }
}
const ray = new THREE.Raycaster();
renderer.domElement.addEventListener('pointerdown', (e) => {
  if (gizmo.dragging) return;
  const r = renderer.domElement.getBoundingClientRect();
  const p = new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1,
                              -((e.clientY - r.top) / r.height) * 2 + 1);
  ray.setFromCamera(p, camera);
  const hit = ray.intersectObjects(pickers.children)[0];
  if (hit) selectBone(hit.object.userData.bone);
});

// ---------- undo/redo ----------
// one snapshot per GESTURE: a whole gizmo drag or one button press is a
// single undo step, captured before the first mutation
const undoStack = [], redoStack = [];
function pushUndo() {
  undoStack.push(JSON.stringify(clip.keys));
  if (undoStack.length > 100) undoStack.shift();
  redoStack.length = 0;
}
function restore(from, to) {
  if (!from.length) { status(`nothing to ${from === undoStack ? 'undo' : 'redo'}`); return; }
  to.push(JSON.stringify(clip.keys));
  clip.keys = JSON.parse(from.pop());
  dirty = true;
  applyFrame(curFrame); refreshBoneInfo();
  status(from === undoStack ? 'undo ↩ (unsaved)' : 'redo ↪ (unsaved)');
}
const undo = () => restore(undoStack, redoStack);
const redo = () => restore(redoStack, undoStack);
document.getElementById('undo').onclick = undo;
document.getElementById('redo').onclick = redo;
addEventListener('keydown', (e) => {
  if (!clip || !(e.ctrlKey || e.metaKey)) return;
  const k = e.key.toLowerCase();
  if (k === 'z' && !e.shiftKey) { e.preventDefault(); undo(); }
  else if (k === 'y' || (k === 'z' && e.shiftKey)) { e.preventDefault(); redo(); }
});

// ---------- editing ----------
const gizmo = new TransformControls(camera, renderer.domElement);
gizmo.setMode('rotate'); gizmo.setSpace('local'); gizmo.setSize(0.7);
scene.add(gizmo.getHelper ? gizmo.getHelper() : gizmo);
gizmo.addEventListener('dragging-changed', (e) => {
  orbit.enabled = !e.value;
  if (e.value && keyAt(curFrame) && selected) pushUndo();
});
gizmo.addEventListener('objectChange', () => { captureBone(); });

function selectBone(n) {
  selected = n;
  document.getElementById('bonesel').value = n;
  gizmo.attach(bones[n]);
  const k = keyAt(curFrame);
  gizmo.enabled = !!k; gizmo.visible = !!k;
  refreshBoneInfo(); updatePickers();
}
document.getElementById('bonesel').onchange = (e) => {
  if (e.target.value) selectBone(e.target.value);
};

function captureBone() {
  const k = keyAt(curFrame);
  if (!k || !selected) return;
  const local = bones[selected].quaternion.clone();
  const pose = restQ[selected].clone().invert().multiply(local);
  if (pose.angleTo(new THREE.Quaternion()) < 0.002) delete k.bones[selected];
  else k.bones[selected] = [pose.w, pose.x, pose.y, pose.z].map((c) => +c.toFixed(5));
  refreshBoneInfo();
  dirty = true;
  status(`edited ${selected} @ f${curFrame} (unsaved)`);
}

function refreshBoneInfo() {
  const el = document.getElementById('boneinfo');
  if (!selected || !clip) { el.textContent = ''; return; }
  const k = keyAt(curFrame);
  const rec = k && k.bones[selected];
  const q = new THREE.Quaternion();
  if (rec) q.set(rec[1], rec[2], rec[3], rec[0]);
  const e = new THREE.Euler().setFromQuaternion(q, 'XYZ');
  const d = (r) => (r * 180 / Math.PI).toFixed(1);
  el.textContent = `${selected}\nlocal XYZ: ${d(e.x)}°, ${d(e.y)}°, ${d(e.z)}°` +
    (rec ? '' : '  (at rest)');
}

document.querySelectorAll('[data-n]').forEach((b) => b.onclick = () => {
  const k = keyAt(curFrame);
  if (!k || !selected) { status('select a bone ON a key first'); return; }
  pushUndo();
  const [axis, sign] = [b.dataset.n[0], b.dataset.n[1] === '+' ? 1 : -1];
  const v = { x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] }[axis];
  const rot = new THREE.Quaternion().setFromAxisAngle(
    new THREE.Vector3(...v), sign * Math.PI / 36);
  const rec = k.bones[selected];
  const q = rec ? new THREE.Quaternion(rec[1], rec[2], rec[3], rec[0])
                : new THREE.Quaternion();
  q.multiply(rot);
  k.bones[selected] = [q.w, q.x, q.y, q.z].map((c) => +c.toFixed(5));
  applyFrame(curFrame); refreshBoneInfo();
  dirty = true;
  status(`nudged ${selected} (unsaved)`);
});

document.getElementById('resetbone').onclick = () => {
  const k = keyAt(curFrame);
  if (k && selected) { pushUndo(); delete k.bones[selected]; applyFrame(curFrame);
    refreshBoneInfo(); dirty = true;
    status(`${selected} reset @ f${curFrame} (unsaved)`); }
};
document.getElementById('copyprev').onclick = () => {
  const k = keyAt(curFrame);
  if (!k || !selected) return;
  const i = clip.keys.indexOf(k);
  if (i > 0) {
    pushUndo();
    const prev = clip.keys[i - 1].bones[selected];
    if (prev) k.bones[selected] = [...prev]; else delete k.bones[selected];
    applyFrame(curFrame); refreshBoneInfo(); dirty = true;
    status('copied (unsaved)');
  }
};

// ---------- transport ----------
document.getElementById('frame').oninput = (e) => setFrame(Number(e.target.value));
document.getElementById('prev').onclick = () => setFrame(Math.max(clip.frame_start, curFrame - 1));
document.getElementById('next').onclick = () => setFrame(Math.min(clip.frame_end, curFrame + 1));
document.getElementById('play').onclick = () => { playing = !playing;
  document.getElementById('play').textContent = playing ? '⏸ stop' : '▶ play'; };
document.querySelectorAll('[data-cam]').forEach((b) => b.onclick = () => setCam(b.dataset.cam));

document.getElementById('save').onclick = async () => {
  const r = await fetch('/save', { method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ file: `${CLIP}.clip.json`, data: clip }) });
  if (r.ok) dirty = false;
  status(r.ok ? 'SAVED ✓ — tell Claude to sync it into the game' : 'save FAILED');
};
document.getElementById('reload').onclick = () => location.reload();

let last = 0;
function tick(t) {
  requestAnimationFrame(tick);
  if (playing && clip) {
    if (t - last > 1000 / clip.fps / 0.5) {   // half-speed playback
      last = t;
      let f = curFrame + 1;
      if (f > clip.frame_end) f = clip.frame_start;
      setFrame(f);
    }
  }
  resize();
  renderer.render(scene, camera);
}
requestAnimationFrame(tick);
