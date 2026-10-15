const api = window.pet;
const host = document.getElementById('pet');
const bubble = document.getElementById('bubble');

const GRAVITY = 2600; // px/s², for falling after being dropped
const DRAG_THRESHOLD = 6; // px the mouse must travel before a press counts as a drag
const NOTICE = 170; // px beyond its own body at which it notices the mouse pointer
const BORED_AFTER = 4; // seconds the pointer may sit still before it loses interest
const TIRED_AFTER = 7; // seconds it grabs at the pointer before giving up for a while
const REST = 5; // seconds it then ignores the pointer

let character; // { id, base, config }
let cfg;
let view; // draws the character: sprite-view.js or model-view.js
let settings; // { scale, stay, sleep, mute, chase }
let win; // window size { w, h }
let area; // work area of the display the pet is on
let pos; // window top-left, in screen px
let sent = { x: NaN, y: NaN };

let state = 'idle';
let stateTime = 2; // seconds left in the current state
let facing = 1;
let targetX = 0;
let fallSpeed = 0;
let fallFrom = 0; // where the current fall started
let sobTimer = 0;
let clicks = []; // timestamps of recent clicks
let tantrums = 0; // times it has snapped since its last black hole
let press = null; // in-progress mouse press on the pet
let bubbleTimer;
let cursor = null; // the mouse pointer on screen: { x, y, movedAt }
let ignorePointerUntil = 0;
let noticedAt = -Infinity;
let others = []; // the other pets on screen: [{ id, x, y, state }], x at their middle
let meeting = null; // { with, kind, lead, time, spoken } while stopped to meet another pet
let comforting = null; // { with, said } while going to a pet that is crying
let comfortAgainAt = 0;

const rand = (min, max) => min + Math.random() * (max - min);
const pick = (list) => list[Math.floor(Math.random() * list.length)];
const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

// The pet is centred in the window, so let the window overhang the screen edge
// by its transparent margin and the pet itself can reach the edge.
const margin = () => (win.w - cfg.size) / 2;
const minX = () => area.x - margin();
const maxX = () => area.x + area.width - win.w + margin();
const minY = () => area.y - (win.h - cfg.size);
const floorY = () => area.y + area.height - win.h;

function setState(next, seconds) {
  // Put to sleep, it dozes off whenever it would otherwise be standing around.
  if (next === 'idle' && settings.sleep) {
    next = 'sleep';
    seconds = Infinity;
  }
  state = next;
  stateTime = seconds;
  view?.setState(next === 'meet' ? meeting.kind : next);
  document.body.classList.toggle('sleeping', next === 'sleep');
  api.state(next); // the other pets are told what this one is up to
}

const middle = () => pos.x + win.w / 2;
// Another pet standing on the same level as this one.
const beside = (other) => Math.abs(other.y - pos.y) < 40;
// How far apart two pets like to stand.
const elbowRoom = () => cfg.size * 0.55 + 32;
// How wide the pet itself is drawn, in pixels (its box is wider than it is).
const bodyWidth = () => cfg.size * (view?.width ?? 0.75);

function say(kind, seconds = 1.6) {
  const line = pick(cfg.lines?.[kind] ?? []);
  if (!line || settings.mute) return;
  bubble.textContent = line;
  bubble.hidden = false;
  bubble.classList.remove('pop');
  void bubble.offsetWidth; // restart the pop animation
  bubble.classList.add('pop');
  clearTimeout(bubbleTimer);
  bubbleTimer = setTimeout(() => (bubble.hidden = true), seconds * 1000);
}

function startWalk() {
  targetX = rand(minX(), maxX());
  // now and then it wanders over to one of the others instead
  const friend = pick(others.filter((other) => beside(other) && other.state !== 'sleep'));
  if (friend && Math.random() < 0.4) {
    const side = Math.random() < 0.5 ? -1 : 1;
    targetX = clamp(friend.x - win.w / 2 + side * elbowRoom() * 1.3, minX(), maxX());
  }
  if (Math.abs(targetX - pos.x) < 40) return setState('idle', rand(1, 3));
  facing = targetX > pos.x ? 1 : -1;
  view?.setFacing(facing);
  setState('walk', Infinity);
}

function startFall() {
  fallSpeed = 0;
  fallFrom = pos.y;
  setState('fall', Infinity);
}

// Coming down from high enough, some characters crack the screen where they land.
function land() {
  const dropped = floorY() - fallFrom;
  pos.y = floorY();
  setState('idle', 1);
  if (!cfg.crack || dropped < (cfg.crack.fromHeight ?? 200)) return;
  if (!settings.sleep) say('land', 1.6);
  const { width = 420, height = 260, seconds = 5 } = cfg.crack;
  api.effect({
    kind: 'crack',
    x: clamp(pos.x + win.w / 2 - width / 2, area.x, area.x + area.width - width),
    y: area.y + area.height - height,
    width,
    height,
    seconds,
  });
}

// The character rips a small black hole open beside itself, on the side with more room.
// The main process draws it in a window of its own.
function openBlackHole() {
  const { size = 260, seconds = 6 } = cfg.blackHole;
  const petX = pos.x + win.w / 2;
  const petY = pos.y + win.h - cfg.size / 2;
  facing = petX < area.x + area.width / 2 ? 1 : -1;
  view?.setFacing(facing);
  api.effect({
    kind: 'blackhole',
    x: clamp(petX + facing * (cfg.size + size) / 2 - size / 2, area.x, area.x + area.width - size),
    y: clamp(petY - size * 0.8, area.y, area.y + area.height - size),
    width: size,
    height: size,
    seconds,
  });
  setState('special', seconds);
  say('special', 2.2);
}

function onClick() {
  if (state === 'annoyed' || state === 'cry' || state === 'special' || state === 'sleep') return;
  const now = performance.now();
  clicks = clicks.filter((t) => now - t < cfg.clickWindowMs);
  clicks.push(now);

  if (clicks.length >= cfg.annoyAfterClicks) {
    clicks = [];
    tantrums++;
    setState('annoyed', 1.8);
    say('annoyed', 1.8);
  } else {
    // one click away from snapping: a first, quieter "no"
    const warn = clicks.length === cfg.annoyAfterClicks - 1 && cfg.lines?.warn;
    setState('talk', 1.2);
    say(warn ? 'warn' : 'click', 1.2);
  }
}

async function drop() {
  host.classList.remove('dragging');
  area = await api.area({ x: pos.x + win.w / 2, y: pos.y + win.h / 2 });
  pos.x = clamp(pos.x, minX(), maxX());
  pos.y = clamp(pos.y, minY(), floorY());
  // Told to stay, it stays right where it was put down, even in mid-air.
  if (settings.stay) setState('idle', rand(1, 3));
  else startFall();
}

// Where the pointer is from the pet's middle, if it is within `range` and worth chasing:
// chasing not switched off, the pointer still being moved about, and the pet not worn out
// from the last chase.
function pointerNearby(range) {
  const now = performance.now();
  if (!settings.chase || !cursor || now < ignorePointerUntil || now - cursor.movedAt > BORED_AFTER * 1000) return null;
  const dx = cursor.x - (pos.x + win.w / 2);
  const dy = cursor.y - (pos.y + win.h - cfg.size / 2);
  return Math.hypot(dx, dy) < range ? { dx, dy } : null;
}

let last = performance.now();
function tick(now) {
  const dt = Math.min((now - last) / 1000, 0.1);
  last = now;
  stateTime -= dt;

  const range = NOTICE + cfg.size / 2;
  if ((state === 'idle' || state === 'walk') && pointerNearby(range)) {
    // spotted the pointer: after it! (told to stay, it can only reach from where it sits)
    setState(settings.stay ? 'reach' : 'chase', TIRED_AFTER);
    if (now - noticedAt > 6000) {
      noticedAt = now;
      say('click', 1.2);
    }
  }

  // someone is crying: go and keep them company (told to stay, only if they are right there)
  if ((state === 'idle' || state === 'walk') && now > comfortAgainAt) {
    const reach = settings.stay ? 170 : 700;
    const sad = others.find((other) => other.state === 'cry' && beside(other) && Math.abs(other.x - middle()) < reach);
    if (sad) {
      comforting = { with: sad.id, said: false };
      setState('comfort', 9);
    }
  }

  switch (state) {
    case 'chase':
    case 'reach': {
      const pointer = pointerNearby(range * 1.3);
      if (!pointer || stateTime <= 0) {
        if (stateTime <= 0) ignorePointerUntil = now + REST * 1000;
        setState('idle', rand(1, 2.5));
        break;
      }
      facing = pointer.dx >= 0 ? 1 : -1;
      view?.setFacing(facing);
      // How close sideways counts as having got there. Under a pointer that is overhead it
      // stands right beneath; one down at its own height it stops just beside, so it does
      // not park itself underneath and take the clicks meant for whatever is there.
      const overhead = pointer.dy < -cfg.size * 0.3;
      const arm = overhead ? cfg.size * 0.3 : bodyWidth() / 2 + 10;
      if (state === 'reach') {
        // it got away sideways: run after it again
        if (!settings.stay && Math.abs(pointer.dx) > arm * 1.8) setState('chase', stateTime);
      } else if (Math.abs(pointer.dx) <= arm) {
        setState('reach', stateTime);
      } else {
        pos.x = clamp(pos.x + facing * cfg.speed * 1.2 * dt, minX(), maxX());
      }
      break;
    }
    case 'meet': {
      const partner = others.find((other) => other.id === meeting.with);
      meeting.time += dt;
      // over, or the other one had something else come up
      if (!partner || stateTime <= 0 || (meeting.time > 0.8 && partner.state !== 'meet')) {
        meeting = null;
        setState('idle', rand(1, 3));
        break;
      }
      const dx = partner.x - middle();
      facing = dx >= 0 ? 1 : -1;
      view?.setFacing(facing);
      // shuffle back if they are standing on top of each other
      if (Math.abs(dx) < elbowRoom() && !settings.stay) pos.x = clamp(pos.x - facing * cfg.speed * 0.7 * dt, minX(), maxX());
      // they take turns: whoever leads speaks first, the other answers a second later
      const turns = { greet: [0.3], chat: [0.3, 2.3], dance: [0.5] }[meeting.kind].map((at) => at + (meeting.lead ? 0 : 1));
      if (meeting.time >= turns[meeting.spoken]) {
        meeting.spoken++;
        say(meeting.kind === 'dance' ? 'click' : 'idle', 1.2);
      }
      break;
    }
    case 'comfort': {
      const sad = others.find((other) => other.id === comforting.with);
      if (!sad || sad.state !== 'cry' || stateTime <= 0) {
        comforting = null;
        comfortAgainAt = now + 8000;
        setState('idle', rand(1, 2));
        break;
      }
      // Stand right up against them, close enough to pat, on the side it is coming from.
      // If another comforter is on that side already, the one earlier by name keeps it and
      // this one goes round to the other side.
      let side = middle() >= sad.x ? 1 : -1;
      if (others.some((other) => other.state === 'comfort' && other.id < character.id && (other.x >= sad.x ? 1 : -1) === side)) {
        side = -side;
      }
      const spot = clamp(sad.x + side * bodyWidth() * 0.95 - win.w / 2, minX(), maxX());
      const walking = Math.abs(spot - pos.x) > 2 && !settings.stay;
      if (walking) {
        facing = spot > pos.x ? 1 : -1;
        pos.x += facing * Math.min(cfg.speed * 1.2 * dt, Math.abs(spot - pos.x));
      } else {
        facing = sad.x >= middle() ? 1 : -1;
      }
      view?.setFacing(facing);
      view?.setState(walking ? 'walk' : 'comfort');
      if (!walking && !comforting.said) {
        comforting.said = true;
        say('comfort', 2);
      }
      break;
    }
    case 'idle':
      if (stateTime <= 0) {
        if (Math.random() < 0.3) {
          setState('talk', 1.6);
          say('idle');
        } else if (settings.stay) {
          setState('idle', rand(2, 5));
        } else {
          startWalk();
        }
      }
      break;
    case 'walk':
      pos.x += cfg.speed * dt * facing;
      if ((targetX - pos.x) * facing <= 0) {
        pos.x = targetX;
        setState('idle', rand(1.5, 4));
      }
      break;
    case 'talk':
    case 'special':
      if (stateTime <= 0) setState('idle', rand(1, 3));
      break;
    case 'annoyed':
      if (stateTime <= 0) {
        if (cfg.blackHole && tantrums >= (cfg.blackHole.afterTantrums ?? 3)) {
          tantrums = 0;
          openBlackHole();
        } else if (cfg.lines?.cry?.length) {
          // characters without cry lines just get over it
          setState('cry', cfg.cryMs / 1000);
          sobTimer = 0;
        } else {
          setState('idle', rand(1, 3));
        }
      }
      break;
    case 'cry':
      sobTimer -= dt;
      if (sobTimer <= 0 && stateTime > 1) {
        sobTimer = 1.7;
        say('cry', 1.5);
      }
      if (stateTime <= 0) setState('idle', rand(2, 4));
      break;
    case 'fall':
      fallSpeed += GRAVITY * dt;
      pos.y += fallSpeed * dt;
      if (pos.y >= floorY()) land();
      break;
  }

  view?.update(dt);

  const x = Math.round(pos.x);
  const y = Math.round(pos.y);
  if (x !== sent.x || y !== sent.y) {
    sent = { x, y };
    api.setPos(x, y);
  }
  requestAnimationFrame(tick);
}

host.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  host.setPointerCapture(e.pointerId);
  press = { sx: e.screenX, sy: e.screenY, px: pos.x, py: pos.y, dragging: false };
  api.hold(true); // the window keeps the mouse until the button is let go
});

host.addEventListener('pointermove', (e) => {
  if (!press) return;
  const dx = e.screenX - press.sx;
  const dy = e.screenY - press.sy;
  if (!press.dragging) {
    if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
    press.dragging = true;
    host.classList.add('dragging');
    bubble.hidden = true;
    setState('drag', Infinity);
  }
  pos.x = press.px + dx;
  pos.y = press.py + dy;
});

// The press is over: a click if it never moved, otherwise the pet is put down.
function release(click) {
  if (!press) return;
  const wasDrag = press.dragging;
  press = null;
  api.hold(false);
  if (wasDrag) drop();
  else if (click) onClick();
}

host.addEventListener('pointerup', () => release(true));
// should the mouse be taken away mid-press (a menu, another window), let go cleanly
host.addEventListener('lostpointercapture', () => release(false));

host.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  api.menu();
});

// The character's own size times the Size setting, kept inside the window.
function applySize() {
  cfg.size = Math.round(Math.min(cfg.baseSize * settings.scale, win.w - 20, win.h - 90));
  document.body.style.setProperty('--size', `${cfg.size}px`);
  view?.resize(cfg.size);
  pos.x = clamp(pos.x, minX(), maxX());
  // the main process needs the pet's outline to know when the mouse is on it
  api.body({ width: bodyWidth(), height: cfg.size * (view?.height ?? 0.8) });
}

function applySettings(next) {
  const released = settings.stay && !next.stay;
  settings = next;
  applySize();
  view?.setSitting(settings.stay);
  if (settings.mute) bubble.hidden = true;
  if (settings.stay && state === 'walk') setState('idle', rand(1, 3));
  if (settings.sleep && (state === 'idle' || state === 'walk' || state === 'talk')) {
    bubble.hidden = true;
    setState('idle');
  } else if (!settings.sleep && state === 'sleep') {
    setState('idle', rand(1, 2));
  }
  if (released && (state === 'idle' || state === 'sleep') && pos.y < floorY()) startFall();
}

async function loadCharacter(next) {
  character = next;
  cfg = {
    size: 150,
    speed: 60,
    annoyAfterClicks: 5,
    clickWindowMs: 4000,
    cryMs: 6000,
    ...next.config,
  };
  cfg.baseSize = cfg.size;
  clicks = [];
  tantrums = 0;
  bubble.hidden = true;
  view?.dispose();
  view = null;
  applySize();

  const { createView } = await import(cfg.model ? './model-view.js' : './sprite-view.js');
  view = await createView(host, next, cfg);
  applySize(); // now that its real proportions are known
  view.setFacing(facing);
  view.setSitting(settings.stay);
  setState('idle', 2);
  view.update(0);

  // Once it is on screen, hand the main process a picture of it for the tray icon.
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      const r = host.getBoundingClientRect();
      api.ready({ x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height) });
    }),
  );
}

api.onSettings(applySettings);
api.onCursor((point) => (cursor = { ...point, movedAt: performance.now() }));
api.onOthers((list) => {
  // one of them just did something alarming nearby: jump and look
  const alarming = list.find((other) => other.state === 'special' && others.find((was) => was.id === other.id)?.state !== 'special');
  others = list;
  if (alarming && (state === 'idle' || state === 'walk') && Math.abs(alarming.x - middle()) < 500) {
    facing = alarming.x >= middle() ? 1 : -1;
    view?.setFacing(facing);
    setState('talk', 1.3);
    say('click', 1.2);
  }
});
api.onMeet((request) => {
  if (state !== 'idle' && state !== 'walk') return; // busy; the other one will notice and move on
  meeting = { ...request, time: 0, spoken: 0 };
  setState('meet', request.seconds);
});
api.onArea((value) => {
  area = value;
  pos.x = clamp(pos.x, minX(), maxX());
  pos.y = settings.stay ? clamp(pos.y, minY(), floorY()) : floorY();
});

api.init().then((init) => {
  ({ settings, pos, area } = init);
  win = init.size;
  requestAnimationFrame(tick);
  return loadCharacter(init.character);
}).catch(console.error);
