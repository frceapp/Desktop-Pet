const { app, BrowserWindow, Menu, Tray, desktopCapturer, ipcMain, screen, shell } = require('electron');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const CHAR_DIR = path.join(__dirname, 'characters');
const WIN_W = 260;
const WIN_H = 300;

const pets = new Map(); // characters now on screen: id -> window
const moods = new Map(); // what each is doing, as it last reported: id -> state
const meetAgain = new Map(); // when a pair may next stop to meet: "a|b" -> time
let tray;
const effects = new Map(); // screen effects now showing: kind -> window
// `characters` are the ids chosen to be on screen; `pets` holds each one's own
// { scale, stay, sleep, mute }, set from its right-click menu.
let settings = { characters: ['chiikawa'], pets: {} };

const SIZES = [
  ['Tiny', 0.6],
  ['Small', 0.8],
  ['Normal', 1],
  ['Large', 1.5],
  ['Huge', 2],
];

function settingsFile() {
  return path.join(app.getPath('userData'), 'settings.json');
}

function loadSettings() {
  try {
    const saved = JSON.parse(fs.readFileSync(settingsFile(), 'utf8'));
    if (Array.isArray(saved.characters)) {
      settings = { characters: saved.characters, pets: saved.pets ?? {} };
    } else if (saved.character) {
      // from before several characters could be out at once
      const { character, scale, stay, sleep } = saved;
      settings = { characters: [character], pets: { [character]: { scale, stay, sleep } } };
    }
  } catch {
    // first run, keep defaults
  }
}

function saveSettings() {
  fs.writeFileSync(settingsFile(), JSON.stringify(settings, null, 2));
}

function petSettings(id) {
  const own = settings.pets[id] ?? {};
  return { scale: own.scale ?? 1, stay: own.stay ?? false, sleep: own.sleep ?? false, mute: own.mute ?? false };
}

function changeSetting(id, key, value) {
  settings.pets[id] = { ...petSettings(id), [key]: value };
  saveSettings();
  pets.get(id)?.webContents.send('pet:settings', petSettings(id));
}

// Every folder in characters/ with a valid character.json is a selectable pet.
function loadCharacters() {
  const list = [];
  for (const entry of fs.readdirSync(CHAR_DIR, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const dir = path.join(CHAR_DIR, entry.name);
    try {
      const config = JSON.parse(fs.readFileSync(path.join(dir, 'character.json'), 'utf8'));
      if (!config.sprites?.idle && !config.model?.file) throw new Error('needs sprites.idle or model.file');
      list.push({ id: entry.name, base: pathToFileURL(dir + path.sep).href, config });
    } catch (err) {
      console.warn(`Skipping character "${entry.name}": ${err.message}`);
    }
  }
  return list;
}

// The chosen characters that still exist; never none, so there is always a pet to right-click.
function chosenCharacters() {
  const all = loadCharacters();
  const chosen = all.filter((c) => settings.characters.includes(c.id));
  return chosen.length ? chosen : all.slice(0, 1);
}

// `id` is the pet that was right-clicked; the tray's menu has none and so no per-pet items.
function buildMenu(id) {
  const chosen = chosenCharacters().map((c) => c.id);
  const own = id ? petSettings(id) : null;
  return Menu.buildFromTemplate([
    {
      label: 'Characters',
      submenu: loadCharacters().map((c) => ({
        label: c.config.name ?? c.id,
        type: 'checkbox',
        checked: chosen.includes(c.id),
        enabled: !(chosen.length === 1 && chosen[0] === c.id), // the last one stays
        click: (item) => {
          settings.characters = item.checked ? [...chosen, c.id] : chosen.filter((other) => other !== c.id);
          saveSettings();
          syncPets();
        },
      })),
    },
    ...(own
      ? [
          {
            label: 'Size',
            submenu: SIZES.map(([label, scale]) => ({
              label,
              type: 'radio',
              checked: scale === own.scale,
              click: () => changeSetting(id, 'scale', scale),
            })),
          },
          {
            label: 'Stay',
            type: 'checkbox',
            checked: own.stay,
            click: (item) => changeSetting(id, 'stay', item.checked),
          },
          {
            label: 'Sleep',
            type: 'checkbox',
            checked: own.sleep,
            click: (item) => changeSetting(id, 'sleep', item.checked),
          },
          {
            label: 'Mute',
            type: 'checkbox',
            checked: own.mute,
            click: (item) => changeSetting(id, 'mute', item.checked),
          },
        ]
      : []),
    { type: 'separator' },
    { label: 'Open characters folder', click: () => shell.openPath(CHAR_DIR) },
    { type: 'separator' },
    { label: 'Quit', click: () => app.quit() },
  ]);
}

function workAreaAt(point) {
  return screen.getDisplayNearestPoint(point).workArea;
}

function workAreaOf(win) {
  const [x, y] = win.getPosition();
  return workAreaAt({ x: x + WIN_W / 2, y: y + WIN_H / 2 });
}

// One window per character, each starting somewhere along the floor.
function createPet(id) {
  const area = screen.getPrimaryDisplay().workArea;
  const win = new BrowserWindow({
    width: WIN_W,
    height: WIN_H,
    x: area.x + Math.round(Math.random() * (area.width - WIN_W)),
    y: area.y + area.height - WIN_H,
    transparent: true,
    frame: false,
    resizable: false,
    hasShadow: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    // Clicking the pet must not steal focus from whatever the user is working in.
    focusable: false,
    show: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      backgroundThrottling: false,
    },
  });
  pets.set(id, win);
  win.on('closed', () => pets.get(id) === win && pets.delete(id) && moods.delete(id));
  // Only the pet itself catches the mouse; the renderer toggles this on hover.
  win.setIgnoreMouseEvents(true, { forward: true });
  win.loadFile(path.join(__dirname, 'renderer', 'index.html'));
  win.once('ready-to-show', () => win.showInactive());
}

// Brings the windows in line with the chosen characters.
function syncPets() {
  const chosen = chosenCharacters().map((c) => c.id);
  for (const id of chosen) {
    if (!pets.has(id)) createPet(id);
  }
  for (const [id, win] of pets) {
    if (!chosen.includes(id)) win.close();
  }
}

// The pet a message came from: [id, window].
function petOf(event) {
  return [...pets].find(([, win]) => win.webContents === event.sender) ?? [];
}

// capturePage rejects until the window has painted its first frame, so try a few times.
async function captureIcon(win, rect) {
  for (let attempt = 0; attempt < 8 && !win.isDestroyed(); attempt++) {
    try {
      const image = (await win.webContents.capturePage(rect)).resize({ width: 32, height: 32 });
      if (!image.isEmpty()) return image;
    } catch {
      // not painted yet
    }
    await new Promise((resolve) => setTimeout(resolve, 400));
  }
  return null;
}

function registerIpc() {
  ipcMain.handle('pet:init', (event) => {
    const [id, win] = petOf(event);
    const [x, y] = win.getPosition();
    return {
      character: loadCharacters().find((c) => c.id === id),
      settings: petSettings(id),
      pos: { x, y },
      size: { w: WIN_W, h: WIN_H },
      area: workAreaOf(win),
    };
  });

  ipcMain.handle('pet:area', (_e, point) => workAreaAt(point));

  // setBounds rather than setPosition: on scaled displays repeated setPosition makes the window grow.
  ipcMain.on('pet:setPos', (event, x, y) => {
    petOf(event)[1]?.setBounds({ x: Math.round(x), y: Math.round(y), width: WIN_W, height: WIN_H });
  });

  ipcMain.on('pet:ignoreMouse', (event, ignore) => {
    petOf(event)[1]?.setIgnoreMouseEvents(ignore, { forward: true });
  });

  ipcMain.on('pet:menu', (event) => {
    const [id, win] = petOf(event);
    if (!win) return;
    // A non-focusable window cannot dismiss its own popup on an outside click.
    win.setFocusable(true);
    win.focus();
    buildMenu(id).popup({ window: win, callback: () => !win.isDestroyed() && win.setFocusable(false) });
  });

  // A pet reports where it is drawn once it has loaded; its picture becomes the tray icon.
  ipcMain.on('pet:ready', async (event, rect) => {
    const [, win] = petOf(event);
    const image = win && (await captureIcon(win, rect));
    if (!image) return;
    if (!tray) {
      tray = new Tray(image);
      tray.setToolTip('Desktop Pet');
      tray.on('click', () => tray.popUpContextMenu(buildMenu()));
      tray.on('right-click', () => tray.popUpContextMenu(buildMenu()));
    } else {
      tray.setImage(image);
    }
  });

  ipcMain.on('pet:effect', (event, request) => openEffect(request, petOf(event)[1]));

  ipcMain.on('pet:state', (event, state) => {
    const [id] = petOf(event);
    if (id) moods.set(id, state);
  });
}

// A picture of what is on screen inside `bounds`, as a data URL. It never leaves this app:
// the black hole bends it so the screen itself seems to warp.
async function grabScreen(bounds) {
  const display = screen.getDisplayMatching(bounds);
  const { width, height } = display.size;
  const sources = await desktopCapturer.getSources({
    types: ['screen'],
    thumbnailSize: { width: Math.round(width * display.scaleFactor), height: Math.round(height * display.scaleFactor) },
  });
  const shot = (sources.find((source) => source.display_id === String(display.id)) ?? sources[0]).thumbnail;
  const perPixel = shot.getSize().width / width;
  return shot
    .crop({
      x: Math.round((bounds.x - display.bounds.x) * perPixel),
      y: Math.round((bounds.y - display.bounds.y) * perPixel),
      width: Math.round(bounds.width * perPixel),
      height: Math.round(bounds.height * perPixel),
    })
    .toDataURL();
}

// Screen effects a character can set off. Each is a page in renderer/ shown in a small
// see-through window for a few seconds. Clicks pass straight through, so an effect only
// disturbs what you see. `screen: true` hands the page a picture of what is underneath it.
const EFFECTS = {
  blackhole: { screen: true },
  crack: { screen: false },
};

async function openEffect({ kind, x, y, width, height, seconds }, pet) {
  if (!EFFECTS[kind] || effects.has(kind)) return;
  const bounds = { x: Math.round(x), y: Math.round(y), width, height };
  const effect = new BrowserWindow({
    ...bounds,
    transparent: true,
    frame: false,
    resizable: false,
    hasShadow: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    focusable: false,
    show: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload-effect.js'),
      backgroundThrottling: false,
    },
  });
  effects.set(kind, effect);
  effect.on('closed', () => effects.delete(kind));
  effect.setIgnoreMouseEvents(true);
  // Without a picture (capture refused or failed) the page still draws what it can.
  const [picture] = await Promise.all([
    EFFECTS[kind].screen ? grabScreen(bounds).catch(() => null) : null,
    effect.loadFile(path.join(__dirname, 'renderer', `${kind}.html`)),
  ]);
  if (effect.isDestroyed()) return;
  effect.webContents.send('effect:start', { picture, seconds });
  effect.showInactive();
  if (pet && !pet.isDestroyed()) pet.moveTop(); // the pet stays in front of its own handiwork
  setTimeout(() => !effect.isDestroyed() && effect.close(), seconds * 1000 + 200);
}

// With more than one pet out, each is kept told where the others are and what they are
// doing, and two that wander close together are told to stop and meet. Deciding that here
// means both hear it at the same moment and agree on what they are doing.
const MEETINGS = { greet: 3.2, chat: 4.6, dance: 4.2 }; // kind -> seconds

function keepCompany() {
  setInterval(() => {
    const all = [...pets]
      .filter(([, win]) => !win.isDestroyed())
      .map(([id, win]) => {
        const [x, y] = win.getPosition();
        return { id, x: x + WIN_W / 2, y, state: moods.get(id) ?? 'idle' };
      });
    // a pet on its own is told so too, or it would go on believing in the ones that left
    for (const pet of all) {
      pets.get(pet.id).webContents.send('pet:others', all.filter((other) => other !== pet));
    }

    const free = (pet) => pet.state === 'idle' || pet.state === 'walk';
    const now = Date.now();
    for (const a of all) {
      for (const b of all) {
        const pair = `${a.id}|${b.id}`;
        if (a.id >= b.id || !free(a) || !free(b) || now < (meetAgain.get(pair) ?? 0)) continue;
        if (Math.abs(a.x - b.x) > 150 || Math.abs(a.y - b.y) > 30) continue;
        const kinds = Object.keys(MEETINGS);
        const kind = kinds[Math.floor(Math.random() * kinds.length)];
        const meeting = { kind, seconds: MEETINGS[kind] };
        pets.get(a.id).webContents.send('pet:meet', { ...meeting, with: b.id, lead: true });
        pets.get(b.id).webContents.send('pet:meet', { ...meeting, with: a.id, lead: false });
        a.state = b.state = 'meet'; // spoken for, should a third be standing right there
        moods.set(a.id, 'meet');
        moods.set(b.id, 'meet');
        meetAgain.set(pair, now + 20000 + Math.random() * 20000);
      }
    }
  }, 200);
}

// Pets chase the mouse, so tell them whenever it moves.
function watchCursor() {
  let last = '';
  setInterval(() => {
    const point = screen.getCursorScreenPoint();
    const now = `${point.x},${point.y}`;
    if (now === last) return;
    last = now;
    for (const win of pets.values()) win.webContents.send('pet:cursor', point);
  }, 60);
}

function sendAreas() {
  for (const win of pets.values()) win.webContents.send('pet:area', workAreaOf(win));
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.whenReady().then(() => {
    loadSettings();
    registerIpc();
    syncPets();
    watchCursor();
    keepCompany();
    screen.on('display-metrics-changed', sendAreas);
    screen.on('display-removed', sendAreas);
  });
  // Effect windows come and go; the app ends when the last pet does.
  app.on('window-all-closed', () => app.quit());
}
