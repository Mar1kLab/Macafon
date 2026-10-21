// Display profile helpers for the Displays screen (§9.10): sketches and edits (DISP-010).
import type { DisplayLayout, DisplayMode, DisplayMonitor, DisplayProfile, DisplayRotation, ProfileMonitor } from '../ipc/protocol';
import { extent, type SketchMonitor } from './LayoutSketch.svelte';

export const sameMode = (a: DisplayMode, b: DisplayMode) =>
  a.width === b.width && a.height === b.height && Math.abs(a.refresh_mhz - b.refresh_mhz) <= 500;

export const modeLabel = (mode: DisplayMode) => `${mode.width} × ${mode.height} · ${Math.round(mode.refresh_mhz / 1000)} Hz`;

/** The connected monitor a profile monitor means (ADR-0006): path, then EDID with serial, then
 * an EDID only one connected monitor has. */
export function connectedOf(monitor: ProfileMonitor, monitors: readonly DisplayMonitor[]): DisplayMonitor | null {
  const { identity } = monitor;
  const byPath = monitors.find((m) => m.identity.path === identity.path);
  if (byPath) return byPath;
  if (identity.serial) {
    const bySerial = monitors.find((m) => m.identity.edid === identity.edid && m.identity.serial === identity.serial);
    if (bySerial) return bySerial;
  }
  const byEdid = monitors.filter((m) => m.identity.edid === identity.edid);
  return byEdid.length === 1 ? byEdid[0]! : null;
}

export function profileSketch(profile: DisplayProfile, monitors: readonly DisplayMonitor[]): SketchMonitor[] {
  return profile.monitors.map((monitor) => ({
    key: monitor.identity.path,
    name: monitor.name,
    layout: monitor.layout,
    missing: connectedOf(monitor, monitors) === null,
  }));
}

export const currentSketch = (monitors: readonly DisplayMonitor[]): SketchMonitor[] =>
  monitors.map((monitor) => ({ key: monitor.identity.path, name: monitor.name, layout: monitor.layout }));

/** Positions moved so `key` is at 0, 0 and the only main monitor. */
export function makePrimary(monitors: ProfileMonitor[], key: string): ProfileMonitor[] {
  const target = monitors.find((monitor) => monitor.identity.path === key)?.layout;
  if (!target) return monitors;
  const [dx, dy] = [target.x, target.y];
  return monitors.map((monitor) => ({
    ...monitor,
    layout: monitor.layout && { ...monitor.layout, x: monitor.layout.x - dx, y: monitor.layout.y - dy, primary: monitor.identity.path === key },
  }));
}

/** A monitor turned on goes right of the others, at `mode`; the first one on becomes main. */
export function turnOn(monitors: ProfileMonitor[], key: string, mode: DisplayMode): ProfileMonitor[] {
  const on = monitors.filter((monitor) => monitor.layout);
  const right = on.reduce((edge, monitor) => Math.max(edge, monitor.layout!.x + extent(monitor.layout!).width), 0);
  const top = on.length ? Math.min(...on.map((monitor) => monitor.layout!.y)) : 0;
  const layout: DisplayLayout = { x: on.length ? right : 0, y: top, mode, rotation: 'none', primary: on.length === 0, hdr: false };
  return monitors.map((monitor) => (monitor.identity.path === key ? { ...monitor, layout } : monitor));
}

/** A monitor turned off; if it was the main one, the leftmost monitor left takes over. Mirrors of
 * it are turned off with it. */
export function turnOff(monitors: ProfileMonitor[], key: string): ProfileMonitor[] {
  const was = monitors.find((monitor) => monitor.identity.path === key)?.layout;
  let next = monitors.map((monitor) =>
    monitor.identity.path === key || monitor.layout?.mirror_of === key ? { ...monitor, layout: null } : monitor,
  );
  if (was?.primary) {
    const left = next.filter((monitor) => monitor.layout).sort((a, b) => a.layout!.x - b.layout!.x)[0];
    if (left) next = makePrimary(next, left.identity.path);
  }
  return next;
}

export function setLayout(monitors: ProfileMonitor[], key: string, patch: Partial<Pick<DisplayLayout, 'mode' | 'rotation' | 'hdr'>>): ProfileMonitor[] {
  return monitors.map((monitor) =>
    monitor.identity.path === key && monitor.layout ? { ...monitor, layout: { ...monitor.layout, ...patch } } : monitor,
  );
}

export const ROTATIONS: DisplayRotation[] = ['none', 'rotate90', 'rotate180', 'rotate270'];
