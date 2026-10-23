import { describe, expect, it } from 'vitest';
import type { SoundInfo } from '../ipc/protocol';
import { FAVORITES, formatDuration, inScope, matches, sortSounds } from './format';

const sound = (name: string, extra: Partial<SoundInfo> = {}): SoundInfo => ({
  id: name,
  name,
  category: null,
  duration_ms: 1000,
  volume_pct: 100,
  favorite: false,
  play_count: 0,
  added_unix_ms: 0,
  error: null,
  ...extra,
});

describe('sounddeck list', () => {
  it('formats durations for short effects and long tracks', () => {
    expect(formatDuration(2400, true)).toBe('0:02.4');
    expect(formatDuration(9999, true)).toBe('0:10.0');
    expect(formatDuration(65_000)).toBe('1:05');
    expect(formatDuration(3_725_000)).toBe('1:02:05');
    expect(formatDuration(null)).toBe('–:––');
  });

  it('filters by words in any order and sorts with numbers in names', () => {
    const memes = sound('Vine boom', { category: 'Memes', favorite: true, play_count: 3 });
    expect(matches(memes, 'memes BOOM')).toBe(true);
    expect(matches(memes, 'airhorn')).toBe(false);
    expect(inScope(memes, FAVORITES)).toBe(true);
    expect(inScope(memes, '')).toBe(false);
    const names = sortSounds([sound('Clip 10'), sound('Clip 2'), memes], 'name').map((s) => s.name);
    expect(names).toEqual(['Clip 2', 'Clip 10', 'Vine boom']);
    expect(sortSounds([sound('a'), memes], 'plays')[0]).toBe(memes);
    const old = sound('Old', { added_unix_ms: 1 });
    const fresh = sound('Fresh', { added_unix_ms: 2 });
    expect(sortSounds([old, fresh], 'newest')[0]).toBe(fresh);
    expect(sortSounds([fresh, old], 'oldest')[0]).toBe(old);
  });
});
