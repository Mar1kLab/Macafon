import { afterEach, expect, it, vi } from 'vitest';
import { DaemonStore } from './daemon.svelte';
import type { DaemonStatus, Transport } from '../ipc/client';
import type { Event as DaemonEvent, MicState } from '../ipc/protocol';
import { BUILTIN_PRESETS } from '../ipc/dsp';

afterEach(() => vi.unstubAllGlobals());

function microphoneHarness() {
  vi.stubGlobal('document', new EventTarget());
  vi.stubGlobal('localStorage', { getItem: () => 'done' });
  const calls: { method: string; params: unknown; resolve: (value: unknown) => void }[] = [];
  const transport: Transport = {
    call: (method, params) => new Promise((resolve) => calls.push({ method, params, resolve })),
    onEvent: () => () => {}, onStatus: () => () => {}, retry: () => {},
  };
  const store = new DaemonStore(transport, true);
  const initial = { device: 'mic-a', active_device: 'mic-a', settings: structuredClone(BUILTIN_PRESETS[0]!.settings) } as MicState;
  store.mic = initial;
  return { store, calls, initial };
}

it.each(['preset', 'device'])('supersedes queued slider edits when selecting a %s', async (selection) => {
  const { store, calls, initial } = microphoneHarness();
  store.edit((s) => { s.output_gain_db = 1; });
  store.edit((s) => { s.output_gain_db = 2; });
  const action = selection === 'preset' ? store.selectPreset('new-preset') :
    store.setMic(store.client.call('mic.set_device', { device: 'mic-b' }), true);
  const selected = { ...initial, device: selection === 'device' ? 'mic-b' : 'mic-a',
    active_device: selection === 'device' ? 'mic-b' : 'mic-a',
    settings: { ...initial.settings, output_gain_db: -6 } };
  // A newer selection can finish before an old response reaches the store.
  calls[1]!.resolve(selected);
  await action;
  calls[0]!.resolve({ ...initial, settings: { ...initial.settings, output_gain_db: 1 } });
  await vi.waitFor(() => expect(store.mic!.settings.output_gain_db).toBe(-6));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(calls).toHaveLength(2);
  expect(store.mic!.device).toBe(selected.device);
});

it('replays an edit made while a preset selection is pending', async () => {
  const { store, calls, initial } = microphoneHarness();
  const action = store.selectPreset('new-preset');
  store.edit((s) => { s.output_gain_db = 4; });
  expect(calls).toHaveLength(1);
  calls[0]!.resolve({ ...initial, preset: 'new-preset', settings: { ...initial.settings, output_gain_db: -6 } });
  await action;
  await vi.waitFor(() => expect(calls).toHaveLength(2));
  expect(calls[1]!.params).toMatchObject({ settings: { output_gain_db: 4 } });
  expect(store.mic!.preset).toBe('new-preset');
});

it('keeps a state event that arrives while the initial snapshot is loading', async () => {
  vi.stubGlobal('document', Object.assign(new EventTarget(), { visibilityState: 'visible' }));
  vi.stubGlobal('localStorage', { getItem: () => 'done' });
  let emit: (event: DaemonEvent) => void = () => {};
  let status: (status: DaemonStatus) => void = () => {};
  const pending: Record<string, (value: unknown) => void> = {};
  const transport: Transport = {
    call: (method) => new Promise((resolve) => {
      if (method === 'hello') return resolve({ capabilities: {} });
      if (method === 'mic.get') pending[method] = resolve;
      else resolve(null);
    }),
    onEvent: (handler) => { emit = handler; return () => {}; },
    onStatus: (handler) => { status = handler; return () => {}; },
    retry: () => {},
  };
  const store = new DaemonStore(transport, false);
  status({ state: 'connected', version: 'test' });
  const settings = structuredClone(BUILTIN_PRESETS[0]!.settings);
  const stale = { device: 'mic-a', active_device: 'mic-a', settings: { ...settings, muted: false } } as MicState;
  const newer = { ...stale, settings: { ...settings, muted: true } };
  await vi.waitFor(() => expect(pending['mic.get']).toBeDefined());
  emit({ event: 'mic.changed', data: newer } as DaemonEvent);
  pending['mic.get']!(stale);
  await vi.waitFor(() => expect(store.hello).not.toBeNull());
  expect(store.mic!.settings.muted).toBe(true);
});

it('does not send undo history from one microphone to another', async () => {
  const { store, calls, initial } = microphoneHarness();
  vi.spyOn(performance, 'now').mockReturnValue(10_000);
  store.edit((s) => { s.output_gain_db = 3; });
  calls[0]!.resolve({ ...initial, settings: { ...initial.settings, output_gain_db: 3 } });
  await vi.waitFor(() => expect(store.canUndo).toBe(true));
  await store.setMic(Promise.resolve({ ...initial, device: 'mic-b', active_device: 'mic-b' }));
  store.undo();
  store.redo();
  expect(store.canUndo).toBe(false);
  expect(store.canRedo).toBe(false);
  expect(calls).toHaveLength(1);
});

it('keeps queued DSP edits when a mute request completes', async () => {
  const { store, calls, initial } = microphoneHarness();
  store.edit((settings) => { settings.output_gain_db = 1; });
  store.edit((settings) => { settings.output_gain_db = 2; });
  await store.setMic(Promise.resolve({ ...initial, settings: { ...initial.settings, muted: true } }));
  expect(store.mic!.settings.output_gain_db).toBe(2);
  expect(store.mic!.settings.muted).toBe(true);
  calls[0]!.resolve({ ...initial, settings: { ...initial.settings, output_gain_db: 1 } });
  await vi.waitFor(() => expect(calls).toHaveLength(2));
  expect(calls[1]!.params).toMatchObject({ settings: { output_gain_db: 2 } });
  calls[1]!.resolve({ ...initial, settings: { ...initial.settings, output_gain_db: 2, muted: true } });
});

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
