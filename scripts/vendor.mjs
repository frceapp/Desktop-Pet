// Copies the few three.js files the app loads from node_modules into renderer/vendor/three,
// so the app runs the same from source and from the installer (which ships no node_modules).
// Run with `npm run vendor` after updating three.

import { cpSync, mkdirSync, rmSync } from 'node:fs';

const from = new URL('../node_modules/three/', import.meta.url);
const to = new URL('../renderer/vendor/three/', import.meta.url);

const FILES = {
  'LICENSE': 'LICENSE',
  'build/three.module.js': 'three.module.js',
  'build/three.core.js': 'three.core.js',
  'examples/jsm/loaders/GLTFLoader.js': 'addons/loaders/GLTFLoader.js',
  'examples/jsm/utils/BufferGeometryUtils.js': 'addons/utils/BufferGeometryUtils.js',
  'examples/jsm/utils/SkeletonUtils.js': 'addons/utils/SkeletonUtils.js',
};

rmSync(to, { recursive: true, force: true });
for (const [source, target] of Object.entries(FILES)) {
  const destination = new URL(target, to);
  mkdirSync(new URL('.', destination), { recursive: true });
  cpSync(new URL(source, from), destination);
  console.log(`copied ${target}`);
}
