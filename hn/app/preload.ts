import { contextBridge, ipcRenderer } from 'electron';

// The upstream engine needs only these two IPC methods for its fetch backend.
contextBridge.exposeInMainWorld('ipcRenderer', {
    invoke: ipcRenderer.invoke.bind(ipcRenderer),
    on: ipcRenderer.on.bind(ipcRenderer),
});
