import { describe, expect, it } from 'vitest';
import { formatVolumeDb, volumeDb } from './format';

describe('channel volume', () => {
  // Mirrors macafon_dsp::channel::volume_db (CH-004): the cubic curve, −∞ at 0%.
  it('follows the cubic curve of the daemon', () => {
    expect(volumeDb(100)).toBe(0);
    expect(volumeDb(50)).toBeCloseTo(60 * Math.log10(0.5), 6);
    expect(volumeDb(30)).toBeCloseTo(-31.37, 2);
    expect(formatVolumeDb(0)).toBe('−∞ dB');
  });
});
