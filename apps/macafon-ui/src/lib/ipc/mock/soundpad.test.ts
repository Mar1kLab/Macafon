import { afterEach, expect, it, vi } from 'vitest';
import { MockSoundpad } from './soundpad';

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it('keeps repeat enabled after stop and subsequent playback', () => {
  vi.useFakeTimers();
  vi.stubGlobal('window', globalThis);
  const soundpad = new MockSoundpad(() => {}, true);
  soundpad.call('soundpad.play', { id: 'Air horn.mp3' });
  soundpad.call('soundpad.set_looping', { looping: true });
  soundpad.call('soundpad.stop', {});
  soundpad.call('soundpad.play', { id: 'Drum roll.wav' });
  expect(soundpad.state.playback?.looping).toBe(true);
  soundpad.call('soundpad.stop', {});
});
