// Draws shattered glass spreading up from the middle of this window's bottom edge, as if
// something heavy had just landed there. Purely a drawing laid over the screen.

const SPREAD = 0.28; // seconds for the cracks to run out
const FADE = 1; // seconds to fade away at the end

const rand = (min, max) => min + Math.random() * (max - min);

// A crack is a jagged line: [[x, y], ...] heading roughly along `angle` for `length`,
// wandering more the farther it gets. Returns its points.
function jagged(x, y, angle, length) {
  const points = [[x, y]];
  for (let gone = 0; gone < length; ) {
    const stride = rand(14, 34);
    angle += rand(-0.32, 0.32);
    x += Math.cos(angle) * stride;
    y -= Math.sin(angle) * stride;
    gone += stride;
    points.push([x, y]);
  }
  return points;
}

// Lays out the whole break once: long cracks fanning up from the impact point, shorter
// ones forking off them, and a couple of rough rings joining neighbours.
function shatter(width, height) {
  const impact = [width / 2, height];
  const reach = Math.min(width / 2, height);
  const cracks = [];
  const spokes = [];
  const count = 11;
  for (let i = 0; i < count; i++) {
    const angle = Math.PI * ((i + rand(0.2, 0.8)) / count);
    // cracks along the floor run farther than ones going straight up
    const spoke = jagged(...impact, angle, reach * rand(0.55, 1) * (0.75 + 0.25 * Math.abs(Math.cos(angle))));
    spokes.push(spoke);
    cracks.push(spoke);
    for (let at = 2; at < spoke.length - 1; at += 2) {
      if (Math.random() < 0.55) cracks.push(jagged(...spoke[at], angle + rand(0.5, 1.1) * (Math.random() < 0.5 ? -1 : 1), reach * rand(0.12, 0.3)));
    }
  }
  for (const step of [1, 2, 4]) {
    for (let i = 0; i + 1 < spokes.length; i++) {
      const a = spokes[i][Math.min(step, spokes[i].length - 1)];
      const b = spokes[i + 1][Math.min(step, spokes[i + 1].length - 1)];
      if (Math.random() < 0.8) cracks.push([a, [(a[0] + b[0]) / 2 + rand(-6, 6), (a[1] + b[1]) / 2 + rand(-6, 6)], b]);
    }
  }
  // loose splinters around the impact catch the light
  const shards = Array.from({ length: 9 }, () => {
    const angle = rand(0.15, Math.PI - 0.15);
    const far = reach * rand(0.08, 0.4);
    const x = impact[0] + Math.cos(angle) * far;
    const y = impact[1] - Math.sin(angle) * far;
    return [[x, y], [x + rand(-22, 22), y - rand(6, 26)], [x + rand(-22, 22), y + rand(-8, 14)]];
  });
  return { impact, reach, cracks, shards };
}

window.effect.onStart(({ seconds }) => {
  const canvas = document.getElementById('effect');
  canvas.width = innerWidth * devicePixelRatio;
  canvas.height = innerHeight * devicePixelRatio;
  const g = canvas.getContext('2d');
  g.scale(devicePixelRatio, devicePixelRatio);
  g.lineCap = g.lineJoin = 'round';
  const { impact, reach, cracks, shards } = shatter(innerWidth, innerHeight);

  const began = performance.now();
  function frame(now) {
    const t = (now - began) / 1000;
    const front = reach * 1.6 * Math.min(1, t / SPREAD); // how far out the break has run
    g.clearRect(0, 0, innerWidth, innerHeight);
    g.globalAlpha = Math.max(0, Math.min(1, (seconds - t) / FADE));

    // only what the spreading front has reached is visible yet
    g.save();
    g.beginPath();
    g.arc(...impact, front, 0, Math.PI * 2);
    g.clip();

    g.fillStyle = 'rgba(255, 255, 255, 0.16)';
    for (const shard of shards) {
      g.beginPath();
      shard.forEach(([x, y], i) => (i ? g.lineTo(x, y) : g.moveTo(x, y)));
      g.fill();
    }
    // each crack is a dark gap with a bright edge beside it, like light catching broken glass
    for (const [color, width, shift] of [['rgba(0, 0, 0, 0.6)', 2.6, 0], ['rgba(255, 255, 255, 0.92)', 1.1, 1.2]]) {
      g.strokeStyle = color;
      g.lineWidth = width;
      g.beginPath();
      for (const crack of cracks) crack.forEach(([x, y], i) => (i ? g.lineTo(x + shift, y - shift) : g.moveTo(x + shift, y - shift)));
      g.stroke();
    }
    g.restore();

    // a quick flash at the moment of impact
    if (t < 0.18) {
      const glow = g.createRadialGradient(...impact, 0, ...impact, reach * 0.7);
      glow.addColorStop(0, `rgba(255, 255, 255, ${0.8 * (1 - t / 0.18)})`);
      glow.addColorStop(1, 'rgba(255, 255, 255, 0)');
      g.fillStyle = glow;
      g.fillRect(0, 0, innerWidth, innerHeight);
    }
    if (t < seconds) requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
});
