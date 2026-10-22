import { LazyTransport } from '../ipc/lazy';
import { TauriTransport, isTauri } from '../ipc/tauri';
import { DaemonStore } from './daemon.svelte';

const tauri = isTauri();

/** `?view=quick-panel`: the quick panel window instead of the main one. */
export const quickPanel = new URLSearchParams(location.search).get('view') === 'quick-panel';

/** In Tauri the bridge reaches the real daemon; in a plain browser the mock daemon runs. */
export const daemon = new DaemonStore(
  tauri ? new TauriTransport() : new LazyTransport(async () => new (await import('../ipc/mock')).MockTransport()),
  !tauri,
  !quickPanel,
);
