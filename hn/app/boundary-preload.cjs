const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('panelProbe', {
    invoke: () => ipcRenderer.invoke('panel-probe:privileged'),
});
