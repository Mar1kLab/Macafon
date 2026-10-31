import { afterEach, expect, it, vi } from 'vitest';
import { MockDictation } from './dictation';
import type { DictationState } from '../protocol';

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it('Linux dictation normalizes imported Windows options and retains its XDG folder on reset', () => {
  vi.useFakeTimers();
  vi.stubGlobal('window', globalThis);
  const mock = new MockDictation(() => {}, 'ready', false);
  const state = mock.call('dictation.set_prefs', { patch: { language: 'layout', gpu: true, game_mode: true } }) as DictationState;
  expect(state.prefs).toMatchObject({ language: 'fixed', gpu: false, game_mode: false });
  expect(state.layout_languages).toEqual([]);
  expect(state.storage.folder).toBe('/home/you/.local/share/macafon/models/stt');
  mock.call('dictation.set_models_dir', { folder: '/home/you/speech' });
  vi.runAllTimers();
  mock.call('dictation.set_models_dir', { folder: null });
  vi.runAllTimers();
  expect(mock.state.storage).toMatchObject({ default_folder: true, folder: state.storage.folder });
});
