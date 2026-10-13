const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('pet', {
  init: () => ipcRenderer.invoke('pet:init'),
  area: (point) => ipcRenderer.invoke('pet:area', point),
  setPos: (x, y) => ipcRenderer.send('pet:setPos', x, y),
  ignoreMouse: (ignore) => ipcRenderer.send('pet:ignoreMouse', ignore),
  menu: () => ipcRenderer.send('pet:menu'),
  ready: (rect) => ipcRenderer.send('pet:ready', rect),
  effect: (request) => ipcRenderer.send('pet:effect', request),
  onSettings: (fn) => ipcRenderer.on('pet:settings', (_e, settings) => fn(settings)),
  onArea: (fn) => ipcRenderer.on('pet:area', (_e, area) => fn(area)),
});
