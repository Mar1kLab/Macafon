import { describe, expect, it, vi } from 'vitest';

type Listener = (message: { payload: unknown }) => void;
const bridge = vi.hoisted(() => ({
  status: { state: 'connecting' } as unknown,
  listeners: [] as Listener[],
  registerListener: () => {},
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (_name: string, listener: Listener) =>
    new Promise((resolve) => {
      // The listener only exists once the backend has registered it.
      bridge.registerListener = () => {
        bridge.listeners.push(listener);
        resolve(() => {});
      };
    }),
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string) => {
    bridge.invoke(command);
    return Promise.resolve(bridge.status);
  },
}));

const { TauriTransport } = await import('./tauri');

describe('TauriTransport.onStatus', () => {
  it('asks for the status only after the listener exists, so an early connect is not lost', async () => {
    const seen: unknown[] = [];
    new TauriTransport().onStatus((status) => seen.push(status));
    await Promise.resolve();
    // The bridge connects while the listener is still being registered: nobody hears the event.
    expect(bridge.invoke).not.toHaveBeenCalled();
    bridge.status = { state: 'connected', version: '1' };
    bridge.registerListener();
    await vi.waitFor(() => expect(seen).toEqual([{ state: 'connected', version: '1' }]));
  });
});
