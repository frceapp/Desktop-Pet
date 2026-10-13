// Shows a character made of one image per state (SVG, PNG, GIF, ...).

export async function createView(host, character, cfg) {
  const img = document.createElement('img');
  img.alt = '';
  img.draggable = false;
  host.replaceChildren(img);

  const s = cfg.sprites;
  let state = 'idle';
  let sitting = false;
  const show = () => {
    const file =
      (sitting && state === 'idle' ? s.sit : null) ??
      s[state] ??
      (['drag', 'fall', 'reach', 'greet', 'chat', 'dance'].includes(state) ? s.talk : null) ??
      (state === 'chase' ? s.walk : null) ??
      (state === 'special' ? s.annoyed : null) ??
      (state === 'sleep' ? s.sit : null) ??
      s.idle;
    const url = character.base + file;
    if (img.src !== url) img.src = url;
  };
  const setState = (next) => {
    state = next;
    show();
  };

  show();
  await img.decode().catch(() => {});

  return {
    setState,
    // sprites are drawn facing right and mirrored when heading left
    setFacing: (dir) => img.style.setProperty('--facing', dir),
    setSitting(value) {
      sitting = value;
      show();
    },
    resize() {},
    update() {},
    dispose: () => img.remove(),
  };
}
