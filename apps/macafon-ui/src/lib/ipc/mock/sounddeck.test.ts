import { afterEach, expect, it, vi } from 'vitest';
import { MockSoundDeck } from './sounddeck';

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it('keeps repeat enabled after stop and subsequent playback', () => {
  vi.useFakeTimers();
  vi.stubGlobal('window', globalThis);
  const sounddeck = new MockSoundDeck(() => {}, true);
  sounddeck.call('sounddeck.play', { id: 'Air horn.mp3' });
  sounddeck.call('sounddeck.set_looping', { looping: true });
  sounddeck.call('sounddeck.stop', {});
  sounddeck.call('sounddeck.play', { id: 'Drum roll.wav' });
  expect(sounddeck.state.playback?.looping).toBe(true);
  sounddeck.call('sounddeck.stop', {});
});
