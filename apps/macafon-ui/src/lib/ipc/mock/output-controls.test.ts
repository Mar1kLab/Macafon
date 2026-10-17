import { afterEach, expect, it, vi } from 'vitest';
import { MockTransport } from '../mock';
import type { OutputControl } from '../protocol';

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it('does not guess the missing half of volume/balance and consumes physical volume events', async () => {
  vi.useFakeTimers();
  vi.stubGlobal('location', { search: '?platform=windows&macaron=live' });
  const transport = new MockTransport();
  await vi.advanceTimersByTimeAsync(300);
  const call = async (method: string, params: unknown = null) => {
    const result = transport.call(method, params);
    // Attach a rejection handler before advancing the asynchronous mock transport.
    const settled = result.then((value) => ({ value, error: null }), (error: Error) => ({ value: null, error }));
    await vi.advanceTimersByTimeAsync(5);
    return settled;
  };
  const before = await call('output_controls.list');
  expect((before.value as OutputControl[])[0]?.settings.volume).toBeNull();
  const edit = await call('output_controls.set', { id: 'mock-macaron', settings: { volume: 50 } });
  expect(edit.error?.message).toContain('together');
  await vi.advanceTimersByTimeAsync(2000);
  const after = await call('output_controls.list');
  expect((after.value as OutputControl[])[0]).toMatchObject({ live_volume: true, settings: { volume: 81, balance: 0, gain: null } });
});
