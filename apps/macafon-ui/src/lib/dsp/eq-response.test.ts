import { describe, expect, it } from 'vitest';
import { bandGainDb } from './eq-response';

// EQ-003: the graph must draw what the SVF does — RBJ prototypes under the bilinear transform.
describe('EQ response', () => {
  it('matches the cookbook at characteristic frequencies', () => {
    expect(bandGainDb({ filter: 'bell', freq_hz: 1000, gain_db: 6, q: 1 }, 1000)).toBeCloseTo(6, 6);
    expect(bandGainDb({ filter: 'bell', freq_hz: 1000, gain_db: -9, q: 4 }, 1000)).toBeCloseTo(-9, 6);
    // Shelves reach their gain far from the corner and half of it at the corner.
    expect(bandGainDb({ filter: 'low_shelf', freq_hz: 200, gain_db: 6, q: 0.707 }, 20)).toBeCloseTo(6, 1);
    expect(bandGainDb({ filter: 'low_shelf', freq_hz: 200, gain_db: 6, q: 0.707 }, 200)).toBeCloseTo(3, 6);
    expect(bandGainDb({ filter: 'high_shelf', freq_hz: 5000, gain_db: -4, q: 0.707 }, 5000)).toBeCloseTo(-2, 6);
    // Butterworth cuts are −3 dB at the corner.
    expect(bandGainDb({ filter: 'low_cut', freq_hz: 100, gain_db: 0, q: Math.SQRT1_2 }, 100)).toBeCloseTo(-3.01, 2);
    expect(bandGainDb({ filter: 'high_cut', freq_hz: 8000, gain_db: 0, q: Math.SQRT1_2 }, 8000)).toBeCloseTo(-3.01, 2);
    expect(bandGainDb({ filter: 'notch', freq_hz: 3000, gain_db: 0, q: 2 }, 3000)).toBeLessThan(-60);
    expect(bandGainDb({ filter: 'bell', freq_hz: 1000, gain_db: 0, q: 1 }, 5000)).toBeCloseTo(0, 9);
  });
});
