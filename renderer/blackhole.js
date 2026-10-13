// Draws a small black hole over a picture of the screen underneath this window, bending
// and smearing that picture so the screen itself seems to be pulled in. The warp fades to
// nothing at the rim, where the picture lines up with the real screen around it.

const GROW = 0.9; // seconds to open
const SHRINK = 0.8; // seconds to close

const VERTEX = `
attribute vec2 corner;
varying vec2 p;
void main() {
  p = corner;
  gl_Position = vec4(corner, 0.0, 1.0);
}`;

const FRAGMENT = `
precision highp float;
uniform sampler2D screen;
uniform float hasScreen;
uniform float time;
uniform float power; // 0 closed .. 1 fully open
varying vec2 p; // -1..1 across the window, y up

vec3 look(vec2 at) {
  return texture2D(screen, clamp(at * 0.5 + 0.5, 0.0, 1.0)).rgb;
}

void main() {
  float r = length(p);
  vec2 out_ = p / max(r, 0.0001);
  float reach = smoothstep(1.0, 0.7, r) * power; // how strongly this spot is affected
  float horizon = 0.2 * power;

  // What shows here is dragged in from farther out and wound around the centre,
  // with a ripple running inward and the odd sideways tear.
  float from = r + reach * (0.1 / (r + 0.1) + 0.02 * sin(r * 26.0 + time * 7.0));
  float wind = reach * (1.4 / (r * 5.0 + 0.5));
  float angle = atan(p.y, p.x) + wind;
  vec2 at = vec2(cos(angle), sin(angle)) * from;
  at.x += reach * 0.03 * step(0.93, sin(p.y * 31.0 + time * 23.0));

  // colours come apart toward the middle
  vec2 split = out_ * reach * 0.03;
  vec3 color = vec3(look(at + split).r, look(at).g, look(at - split).b);

  float shade = smoothstep(horizon, horizon * 2.8, r); // light dies near the hole
  float ring = exp(-pow((r - horizon * 1.45) / (horizon * 0.3 + 0.0001), 2.0)) * power;
  float flare = 0.65 + 0.35 * sin(angle * 3.0 - time * 9.0);
  vec3 glow = mix(vec3(1.0, 0.45, 0.1), vec3(1.0, 0.95, 0.8), ring) * ring * flare;

  float inside = step(horizon, r);
  // with no picture of the screen, only the hole and its halo are drawn
  float alpha = mix(max(1.0 - shade, ring), smoothstep(1.0, 0.94, r), hasScreen);
  color = color * hasScreen * shade * inside + glow * inside;
  gl_FragColor = vec4(color * alpha, alpha);
}`;

function start({ seconds }, image) {
  const canvas = document.getElementById('hole');
  canvas.width = innerWidth * devicePixelRatio;
  canvas.height = innerHeight * devicePixelRatio;
  const gl = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: true });

  const program = gl.createProgram();
  for (const [type, source] of [[gl.VERTEX_SHADER, VERTEX], [gl.FRAGMENT_SHADER, FRAGMENT]]) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
    gl.attachShader(program, shader);
  }
  gl.linkProgram(program);
  gl.useProgram(program);

  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  const corner = gl.getAttribLocation(program, 'corner');
  gl.enableVertexAttribArray(corner);
  gl.vertexAttribPointer(corner, 2, gl.FLOAT, false, 0, 0);

  gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
  for (const side of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T]) gl.texParameteri(gl.TEXTURE_2D, side, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  if (image) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
  else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(4));
  gl.uniform1f(gl.getUniformLocation(program, 'hasScreen'), image ? 1 : 0);

  const time = gl.getUniformLocation(program, 'time');
  const power = gl.getUniformLocation(program, 'power');
  const began = performance.now();
  function frame(now) {
    const t = (now - began) / 1000;
    // swell open, hold, then collapse
    const open = Math.min(1, t / GROW, (seconds - t) / SHRINK);
    gl.uniform1f(time, t);
    gl.uniform1f(power, Math.max(0, open) ** 0.6);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    if (t < seconds) requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
}

window.effect.onStart((options) => {
  if (!options.picture) return start(options, null);
  const image = new Image();
  image.onload = () => start(options, image);
  image.onerror = () => start(options, null);
  image.src = options.picture;
});
