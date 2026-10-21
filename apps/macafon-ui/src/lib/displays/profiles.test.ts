import { describe, expect, it } from 'vitest';
import type { DisplayMonitor, ProfileMonitor } from '../ipc/protocol';
import { connectedOf, makePrimary, turnOff, turnOn } from './profiles';

const mode = (width: number, height: number) => ({ width, height, refresh_mhz: 60_000 });
const monitor = (path: string, edid: string, layout: ProfileMonitor['layout'], serial?: string): ProfileMonitor => ({
  identity: { edid, path, ...(serial ? { serial } : {}) },
  name: path,
  layout,
});

describe('display profile edits', () => {
  it('moves every position so the new main monitor is at 0, 0', () => {
    const next = makePrimary(
      [
        monitor('a', 'A', { x: 0, y: 0, mode: mode(1920, 1080), rotation: 'none', primary: true, hdr: false }),
        monitor('b', 'B', { x: 1920, y: -200, mode: mode(2560, 1440), rotation: 'none', primary: false, hdr: false }),
      ],
      'b',
    );
    expect(next.map((m) => [m.layout!.x, m.layout!.y, m.layout!.primary])).toEqual([[-1920, 200, false], [0, 0, true]]);
  });

  it('turns a monitor on right of the others and hands over the main role when it goes off', () => {
    let monitors = [
      monitor('a', 'A', { x: 0, y: 0, mode: mode(1920, 1080), rotation: 'rotate90', primary: true, hdr: false }),
      monitor('b', 'B', null),
    ];
    monitors = turnOn(monitors, 'b', mode(2560, 1440));
    // A portrait 1080p monitor is 1080 wide.
    expect(monitors[1]!.layout).toMatchObject({ x: 1080, y: 0, primary: false });
    monitors = turnOff(monitors, 'a');
    expect(monitors[0]!.layout).toBeNull();
    expect(monitors[1]!.layout).toMatchObject({ x: 0, y: 0, primary: true });
  });

  it('finds a monitor that moved to another port by its EDID, but not between twins', () => {
    const connected = (path: string, edid: string, serial?: string): DisplayMonitor => ({
      identity: { edid, path, ...(serial ? { serial } : {}) },
      name: path,
      built_in: false,
      layout: null,
      modes: [],
      hdr_supported: false,
    });
    const saved = monitor('old-port', 'GSM5B7F', null, 'S1');
    expect(connectedOf(saved, [connected('new-port', 'GSM5B7F', 'S1')])?.identity.path).toBe('new-port');
    const twin = monitor('old-port', 'DEL4123', null);
    expect(connectedOf(twin, [connected('p1', 'DEL4123'), connected('p2', 'DEL4123')])).toBeNull();
  });
});
