const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('effect', {
  onStart: (fn) => ipcRenderer.once('effect:start', (_e, options) => fn(options)),
});
