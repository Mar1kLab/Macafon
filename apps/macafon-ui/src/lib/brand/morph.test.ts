import { describe, expect, it } from 'vitest';
import { warp } from './morph';

describe('rubber microphone', () => {
  it('keeps the foot still while progressively deforming the shell and grille', () => {
    expect(warp(48, 113, 1)).toEqual([48, 113]);
    expect(warp(48, 10, 0)).toEqual([48, 10]);
    expect(warp(48, 10, 1)).toEqual([100, 42]);
    expect(warp(48, 10, 0.5)).toEqual([74, 26]);
    const top = warp(48, 10, 1);
    const grille = warp(48, 45, 1);
    const holder = warp(48, 93, 1);
    expect(top[0] - 48).toBeGreaterThan(grille[0] - 48);
    expect(grille[0] - 48).toBeGreaterThan(holder[0] - 48);
  });
});
