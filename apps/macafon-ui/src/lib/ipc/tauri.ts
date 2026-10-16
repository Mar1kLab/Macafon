// Transport through the Tauri bridge (src-tauri/src/bridge.rs) to the daemon socket.

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DaemonError, type ClientErrorCode, type DaemonStatus, type Transport } from './client';
import type { Event } from './protocol';

interface BridgeError {
  code: ClientErrorCode;
  message: string;
}

function isBridgeError(value: unknown): value is BridgeError {
  return typeof value === 'object' && value !== null && 'code' in value && 'message' in value;
}

export function isTauri(): boolean {
  return '__TAURI_INTERNALS__' in window;
}

/** The official VB-CABLE download page in the user's browser (WIN-MIC-002). */
export const VB_CABLE_URL = 'https://vb-audio.com/Cable/';

export async function openVbCablePage(): Promise<void> {
  if (isTauri()) await invoke('open_vb_cable_page');
  else window.open(VB_CABLE_URL, '_blank', 'noopener');
}

export class TauriTransport implements Transport {
  async call(method: string, params: unknown): Promise<unknown> {
    try {
      return await invoke('daemon_call', { method, params });
    } catch (error) {
      if (isBridgeError(error)) throw new DaemonError(error.code, error.message);
      throw new DaemonError('internal', String(error));
    }
  }

  onEvent(handler: (event: Event) => void): () => void {
    const unlisten = listen<Event>('daemon://event', (message) => handler(message.payload));
    return () => void unlisten.then((stop) => stop());
  }

  onStatus(handler: (status: DaemonStatus) => void): () => void {
    const unlisten = listen<DaemonStatus>('daemon://status', (message) => handler(message.payload));
    // The bridge may have connected before the listener existed.
    void invoke<DaemonStatus>('daemon_status').then(handler);
    return () => void unlisten.then((stop) => stop());
  }

  retry(): void {
    void invoke('daemon_retry');
  }
}
