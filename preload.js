const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('pet', {
  init: () => ipcRenderer.invoke('pet:init'),
  area: (point) => ipcRenderer.invoke('pet:area', point),
  setPos: (x, y) => ipcRenderer.send('pet:setPos', x, y),
  body: (size) => ipcRenderer.send('pet:body', size),
  hold: (held) => ipcRenderer.send('pet:hold', held),
  menu: () => ipcRenderer.send('pet:menu'),
  ready: (rect) => ipcRenderer.send('pet:ready', rect),
  effect: (request) => ipcRenderer.send('pet:effect', request),
  state: (state) => ipcRenderer.send('pet:state', state),
  onSettings: (fn) => ipcRenderer.on('pet:settings', (_e, settings) => fn(settings)),
  onArea: (fn) => ipcRenderer.on('pet:area', (_e, area) => fn(area)),
  onCursor: (fn) => ipcRenderer.on('pet:cursor', (_e, point) => fn(point)),
  onOthers: (fn) => ipcRenderer.on('pet:others', (_e, others) => fn(others)),
  onMeet: (fn) => ipcRenderer.on('pet:meet', (_e, meeting) => fn(meeting)),
});
