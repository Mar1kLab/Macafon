import { afterEach, expect, it, vi } from 'vitest';
import type { DictationResult } from '../ipc/protocol';

const mocks = vi.hoisted(() => ({ call: vi.fn(), run: vi.fn() }));
vi.mock('../state', () => ({ daemon: { client: { call: mocks.call }, run: mocks.run } }));
import { Copier } from './history.svelte';

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); vi.clearAllMocks(); });

it('copies by daemon result ID without exposing text to the browser clipboard', async () => {
  vi.useFakeTimers();
  const browserCopy = vi.fn();
  vi.stubGlobal('navigator', { clipboard: { writeText: browserCopy } });
  mocks.call.mockResolvedValue(null);
  mocks.run.mockImplementation((request: Promise<unknown>) => request);
  const copier = new Copier();
  await copier.copy({ id: 42, text: 'private transcript' } as DictationResult);
  expect(mocks.call).toHaveBeenCalledWith('dictation.copy', { id: 42 });
  expect(browserCopy).not.toHaveBeenCalled();
  expect(copier.id).toBe(42);
  await vi.advanceTimersByTimeAsync(1500);
  expect(copier.id).toBeNull();
  // The shared run handler reports a failed IPC operation and returns undefined.
  mocks.run.mockResolvedValue(undefined);
  await copier.copy({ id: 43 } as DictationResult);
  expect(copier.id).toBeNull();
});
