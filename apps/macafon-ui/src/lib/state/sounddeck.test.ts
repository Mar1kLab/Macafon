import { afterEach, expect, it, vi } from 'vitest';
import { DaemonStore } from './daemon.svelte';
import { MockSoundDeck } from '../ipc/mock/sounddeck';
import type { Event, SoundDeckPrefsPatch, SoundDeckState } from '../ipc/protocol';
import type { Transport } from '../ipc/client';
import { SOUNDDECK_UPLOAD_CHUNK_BYTES } from '../ipc/protocol';

afterEach(() => vi.unstubAllGlobals());

function setup() {
  vi.stubGlobal('localStorage', { getItem: () => null });
  vi.stubGlobal('document', { addEventListener: () => {} });
  let emit: (event: Event) => void = () => {};
  const call = vi.fn<Transport['call']>();
  const transport: Transport = {
    call,
    onEvent: (handler) => { emit = handler; return () => {}; },
    onStatus: () => () => {},
    retry: () => {},
  };
  const store = new DaemonStore(transport, true, false);
  store.sounddeck = new MockSoundDeck(() => {}, true).state;
  return { store, call, emit: (state: SoundDeckState) => emit({ event: 'sounddeck.changed', data: state }) };
}

it('coalesces prefs without losing concurrent device updates or server clamping', async () => {
  const { store, call, emit } = setup();
  const initial = structuredClone(store.sounddeck!);
  const replies: ((state: SoundDeckState) => void)[] = [];
  call.mockImplementation(() => new Promise((resolve) => replies.push(resolve)));
  store.setSoundDeckPrefs({ headphones_volume_pct: 140 });
  store.setSoundDeckPrefs({ headphones_volume_pct: 170, to_microphone: false });
  emit({ ...initial, prefs: { ...initial.prefs, monitor_device: 'new-device' } });
  expect(store.sounddeck!.prefs.monitor_device).toBe('new-device');
  expect(store.sounddeck!.prefs.headphones_volume_pct).toBe(170);
  replies[0]!({ ...initial, prefs: { ...initial.prefs, monitor_device: 'new-device', headphones_volume_pct: 140 } });
  await vi.waitFor(() => expect(call).toHaveBeenCalledTimes(2));
  expect(call.mock.calls[1]![1]).toEqual({ patch: { headphones_volume_pct: 170, to_microphone: false } satisfies SoundDeckPrefsPatch });
  replies[1]!({ ...initial, prefs: { ...initial.prefs, monitor_device: 'new-device', headphones_volume_pct: 165, to_microphone: false } });
  await vi.waitFor(() => expect(store.sounddeck!.prefs.headphones_volume_pct).toBe(165));
  expect(store.sounddeck!.prefs.monitor_device).toBe('new-device');
});

it('reads and encodes large uploads one bounded chunk at a time', async () => {
  const { store, call } = setup();
  const bytes = Uint8Array.from({ length: SOUNDDECK_UPLOAD_CHUNK_BYTES * 2 + 7 }, (_, i) => i % 256);
  const slice = vi.fn((start: number, end: number) => new Blob([bytes.subarray(start, end)]));
  const file = { name: 'large.wav', size: bytes.length, slice, arrayBuffer: () => { throw new Error('whole file read'); } } as unknown as File;
  const received: number[] = [];
  call.mockImplementation(async (_method, params) => {
    const chunk = params as { data: string; offset: number; done: boolean };
    expect(chunk.offset).toBe(received.length);
    const decoded = Uint8Array.from(atob(chunk.data), (c) => c.charCodeAt(0));
    expect(decoded.length).toBeLessThanOrEqual(SOUNDDECK_UPLOAD_CHUNK_BYTES);
    for (const byte of decoded) received.push(byte);
    expect(chunk.done).toBe(received.length === bytes.length);
    return null;
  });
  const added = await store.uploadSounds([file], null);
  expect(store.toasts).toEqual([]);
  expect(added).toBe(1);
  expect(Uint8Array.from(received)).toEqual(bytes);
  expect(slice).toHaveBeenCalledTimes(3);
  expect(store.uploads).toEqual({});
});


