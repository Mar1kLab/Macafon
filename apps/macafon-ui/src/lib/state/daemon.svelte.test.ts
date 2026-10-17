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
