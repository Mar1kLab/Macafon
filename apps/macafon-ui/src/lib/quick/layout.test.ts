import { describe, expect, it } from 'vitest';
import { movePart, overlayOrder, overlayRows, volumesFirst } from './layout';

describe('overlay layout', () => {
  it('drops unknown and repeated parts and appends missing ones', () => {
    const order = overlayOrder(['preset', 'preset', 'bogus' as never, 'volumes']);
    expect(order.slice(0, 2)).toEqual(['preset', 'volumes']);
    expect(order).toHaveLength(9);
  });

  it('groups neighbouring switches into rows and gives other parts their own', () => {
    expect(overlayRows(['mic_switch', 'sounds', 'preset', 'mixer_switch', 'volumes'])).toEqual([
      { kind: 'switches', parts: ['mic_switch', 'sounds'] },
      { kind: 'wide', part: 'preset' },
      { kind: 'switches', parts: ['mixer_switch'] },
    ]);
  });

  it('puts volumes left only when it comes before every control', () => {
    expect(volumesFirst(['volumes', 'preset'])).toBe(true);
    expect(volumesFirst(['preset', 'volumes'])).toBe(false);
  });

  it('moves a part before or after another, or to an end', () => {
    expect(movePart(['volumes', 'mic_switch', 'preset'], 'preset', 'mic_switch')).toEqual(['volumes', 'preset', 'mic_switch']);
    expect(movePart(['volumes', 'mic_switch', 'preset'], 'volumes', 'preset', true)).toEqual(['mic_switch', 'preset', 'volumes']);
    expect(movePart(['volumes', 'mic_switch', 'preset'], 'volumes', 'end')).toEqual(['mic_switch', 'preset', 'volumes']);
  });
});
