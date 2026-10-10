import { MockTransport } from '../ipc/mock';
import { TauriTransport, isTauri } from '../ipc/tauri';
import { DaemonStore } from './daemon.svelte';

const tauri = isTauri();

/** In Tauri the bridge reaches the real daemon; in a plain browser the mock daemon runs. */
export const daemon = new DaemonStore(tauri ? new TauriTransport() : new MockTransport(), !tauri);
