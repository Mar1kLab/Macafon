import { describe, expect, it } from 'vitest';
import { Needle, vuToTravel } from './needle';

describe('VU needle', () => {
  it('reaches 99 % of a step in 300 ms and overshoots about 1.5 %', () => {
    const needle = new Needle();
    const dt = 1 / 1000;
    let peak = 0;
    let settledAt = Infinity;
    for (let ms = 1; ms <= 1000; ms++) {
      const position = needle.step(1, dt);
      peak = Math.max(peak, position);
      if (settledAt === Infinity && position >= 0.99) settledAt = ms;
    }
    expect(settledAt).toBeLessThanOrEqual(300);
    expect(peak - 1).toBeGreaterThan(0.01);
    expect(peak - 1).toBeLessThan(0.02);
  });

  it('puts 0 VU where a real meter has it, about two thirds of the travel', () => {
    expect(vuToTravel(-20)).toBe(0);
    expect(vuToTravel(3)).toBeCloseTo(1);
    expect(vuToTravel(0)).toBeCloseTo(0.685, 2);
  });
});
