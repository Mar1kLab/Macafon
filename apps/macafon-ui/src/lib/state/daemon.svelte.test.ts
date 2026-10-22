import { afterEach, expect, it, vi } from 'vitest';
import { DaemonStore } from './daemon.svelte';
import type { Transport } from '../ipc/client';

afterEach(() => vi.unstubAllGlobals());

it('suspends device meters while hidden and resumes only while watched', () => {
  const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
  vi.stubGlobal('document', page);
  vi.stubGlobal('localStorage', { getItem: () => 'done' });
  const call = vi.fn(async (_method: string) => null);
  const transport: Transport = {
    call,
    onEvent: () => () => {},
    onStatus: () => () => {},
    retry: () => {},
  };
  const store = new DaemonStore(transport, true);
  store.status = { state: 'connected', version: 'test' };
  const stop = store.watchDevices();
  const stopSecond = store.watchDevices();
  const requests = () => call.mock.calls.map(([method]) => method).filter((method) => String(method).startsWith('devices.'));
  expect(requests()).toEqual(['devices.subscribe']);

  page.visibilityState = 'hidden';
  page.dispatchEvent(new Event('visibilitychange'));
  expect(requests()).toEqual(['devices.subscribe', 'devices.unsubscribe']);
  page.visibilityState = 'visible';
  page.dispatchEvent(new Event('visibilitychange'));
  stopSecond();
  expect(requests()).toEqual(['devices.subscribe', 'devices.unsubscribe', 'devices.subscribe']);
  stop();
  stop();
  expect(requests()).toEqual(['devices.subscribe', 'devices.unsubscribe', 'devices.subscribe', 'devices.unsubscribe']);
});

it('keeps every rapid overlay change when replies arrive late', async () => {
  vi.stubGlobal('document', Object.assign(new EventTarget(), { visibilityState: 'visible' }));
  vi.stubGlobal('localStorage', { getItem: () => 'done' });
  const replies: ((value: unknown) => void)[] = [];
  const sent: unknown[] = [];
  const transport: Transport = {
    call: (method, params) => {
      if (method !== 'prefs.set') return Promise.resolve(null);
      sent.push(params);
      return new Promise((resolve) => replies.push(resolve));
    },
    onEvent: () => () => {},
    onStatus: () => () => {},
    retry: () => {},
  };
  const store = new DaemonStore(transport, false);
  type Prefs = NonNullable<typeof store.prefs>;
  const initial = { volume_step_pct: 5, overlay: { mic_level: true, preset: true } } as unknown as Prefs;
  store.prefs = initial;
  // As Settings does: each change is a whole overlay built from the current prefs.
  const hide = (part: 'mic_level' | 'preset') =>
    store.setPrefs({ overlay: { ...store.prefs!.overlay, [part]: false } });
  hide('mic_level');
  hide('preset');
  // The first reply knows only the first change; the second change must survive it.
  replies[0]!({ ...initial, overlay: { mic_level: false, preset: true } });
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(store.prefs!.overlay).toEqual({ mic_level: false, preset: false });
  expect((sent[1] as { overlay: unknown }).overlay).toEqual({ mic_level: false, preset: false });
});
