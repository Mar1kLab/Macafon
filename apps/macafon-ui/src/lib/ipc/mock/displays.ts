// Display profiles of the mock daemon (PRD §8.5): a laptop panel, a 1440p HDR monitor and a TV
// that is off, saved profiles with a missing monitor, and the 15 s "Keep these display settings?"
// countdown. `?displays=empty` starts with no profiles (the first-run card), `?displays=pending`
// with a switch waiting for Keep.
import { DaemonError } from '../client';
import type {
  DisplayLayout,
  DisplayMonitor,
  DisplayProfile,
  DisplaysState,
  Event,
  ProfileMonitor,
} from '../protocol';
import { clone, slugify } from './helpers';

const CONFIRM_MS = 15_000;

const mode = (width: number, height: number, hz: number) => ({ width, height, refresh_mhz: hz * 1000 });

const LAPTOP: DisplayMonitor = {
  identity: { edid: 'BOE0A1B', path: 'laptop' },
  name: 'Built-in display',
  built_in: true,
  layout: null,
  modes: [mode(2560, 1600, 165), mode(2560, 1600, 60), mode(1920, 1200, 165), mode(1920, 1200, 60)],
  hdr_supported: false,
};
const LG: DisplayMonitor = {
  identity: { edid: 'GSM5B7F', serial: '208NTAB1C', path: 'lg' },
  name: 'LG ULTRAGEAR',
  built_in: false,
  layout: null,
  modes: [mode(2560, 1440, 165), mode(2560, 1440, 144), mode(2560, 1440, 60), mode(1920, 1080, 165), mode(1920, 1080, 60)],
  hdr_supported: true,
};
const TV: DisplayMonitor = {
  identity: { edid: 'SAM7310', path: 'tv' },
  name: 'Samsung TV',
  built_in: false,
  layout: null,
  modes: [mode(3840, 2160, 60), mode(3840, 2160, 30), mode(1920, 1080, 60)],
  hdr_supported: true,
};

const layout = (x: number, y: number, m: ReturnType<typeof mode>, primary = false, hdr = false): DisplayLayout => ({
  x, y, mode: m, rotation: 'none', primary, hdr,
});

const at = (monitor: DisplayMonitor, value: DisplayLayout | null): ProfileMonitor => ({
  identity: clone(monitor.identity), name: monitor.name, layout: value,
});

/** The desk as it is at start: laptop on the left, the LG primary, the TV off. */
const WORK: ProfileMonitor[] = [
  at(LAPTOP, layout(-2560, 0, LAPTOP.modes[1]!)),
  at(LG, layout(0, 0, LG.modes[0]!, true)),
  at(TV, null),
];

const sameLayout = (a: DisplayLayout | null, b: DisplayLayout | null) =>
  a === null || b === null
    ? a === b
    : a.x === b.x && a.y === b.y && a.primary === b.primary && a.rotation === b.rotation && a.hdr === b.hdr
      && a.mode.width === b.mode.width && a.mode.height === b.mode.height
      && Math.abs(a.mode.refresh_mhz - b.mode.refresh_mhz) <= 500;

export class MockDisplays {
  private monitors: DisplayMonitor[] = [LAPTOP, LG, TV].map((monitor) => clone(monitor));
  private profiles: DisplayProfile[] = [];
  private confirmed = new Set<string>();
  private previous: ProfileMonitor[] | null = null;
  private pending: { profile: string | null; revert_at_unix_ms: number } | null = null;
  private revertTimer: number | undefined;

  constructor(
    private readonly emit: (event: Event) => void,
    mode: string | null,
  ) {
    this.set(WORK);
    if (mode !== 'empty') {
      this.profiles = [
        { id: 'work', name: 'Work', icon: 'monitors', monitors: clone(WORK) },
        {
          id: 'game',
          name: 'Game',
          icon: 'gamepad',
          monitors: [at(LAPTOP, null), at(LG, layout(0, 0, LG.modes[0]!, true, true)), at(TV, null)],
        },
        {
          id: 'movie',
          name: 'Movie',
          icon: 'film',
          monitors: [at(LAPTOP, null), at(LG, null), at(TV, layout(0, 0, TV.modes[0]!, true, true))],
          output_device: 'Samsung TV (HDMI)',
        },
        {
          id: 'laptop',
          name: 'Laptop only',
          icon: 'laptop',
          monitors: [
            at(LAPTOP, layout(0, 0, LAPTOP.modes[0]!, true)),
            { identity: { edid: 'DEL4123', path: 'dell' }, name: 'DELL P2419H', layout: null },
            at(LG, null),
          ],
        },
      ];
      for (const id of ['work', 'game', 'laptop']) this.confirmed.add(id);
    }
    if (mode === 'pending') this.apply('movie');
  }

  get state(): DisplaysState {
    const connected = new Set(this.monitors.map((monitor) => monitor.identity.path));
    return clone({
      monitors: this.monitors,
      profiles: this.profiles.map((profile) => ({
        profile,
        missing: profile.monitors.filter((monitor) => !connected.has(monitor.identity.path)).map((monitor) => monitor.name),
      })),
      active: this.profiles.find((profile) => this.matches(profile.monitors))?.id ?? null,
      pending: this.pending,
      can_revert: this.previous !== null,
    });
  }

  call(method: string, p: Record<string, unknown>): unknown {
    switch (method) {
      case 'displays.get':
        return this.state;
      case 'displays.suggestions':
        return this.suggestions();
      case 'displays.save':
        return this.add({ id: '', name: String(p.name), icon: String(p.icon), monitors: this.capture() });
      case 'displays.create':
        return this.add(p.profile as DisplayProfile);
      case 'displays.update': {
        const next = p.profile as DisplayProfile;
        const index = this.index(next.id);
        this.checkName(next.name, next.id);
        this.profiles[index] = clone(next);
        return this.changed();
      }
      case 'displays.recapture':
        this.profiles[this.index(String(p.id))]!.monitors = this.capture();
        this.confirmed.add(String(p.id));
        return this.changed();
      case 'displays.delete':
        this.profiles.splice(this.index(String(p.id)), 1);
        return this.changed();
      case 'displays.reorder': {
        const ids = p.ids as string[];
        this.profiles.sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id));
        return this.changed();
      }
      case 'displays.apply':
        return this.apply(String(p.id));
      case 'displays.confirm':
        if (!this.pending) return this.state;
        if (p.keep) {
          if (this.pending.profile) this.confirmed.add(this.pending.profile);
          this.pending = null;
          clearTimeout(this.revertTimer);
          return this.changed();
        }
        return this.revert();
      case 'displays.revert':
        return this.revert();
      case 'displays.all_on':
        this.previous = this.capture();
        this.monitors.forEach((monitor, i) => {
          monitor.layout = layout(i === 0 ? 0 : this.monitors.slice(0, i).reduce((x, m) => x + m.modes[0]!.width, 0), 0, monitor.modes[0]!, i === 0);
        });
        return this.changed();
      default:
        throw new DaemonError('bad_request', `unknown method ${method}`);
    }
  }

  /** The profile ids, for hotkeys of the mock. */
  get ids(): string[] {
    return this.profiles.map((profile) => profile.id);
  }

  private apply(id: string): DisplaysState {
    const profile = this.profiles[this.index(id)]!;
    if (!profile.monitors.some((monitor) => monitor.layout && this.monitors.some((m) => m.identity.path === monitor.identity.path))) {
      throw new DaemonError('invalid_params', 'none of this profile’s monitors are connected');
    }
    if (!this.pending) this.previous = this.capture();
    this.set(profile.monitors);
    clearTimeout(this.revertTimer);
    if (this.confirmed.has(id)) {
      this.pending = null;
    } else {
      this.pending = { profile: id, revert_at_unix_ms: Date.now() + CONFIRM_MS };
      this.revertTimer = window.setTimeout(() => this.revert(), CONFIRM_MS);
    }
    return this.changed();
  }

  private revert(): DisplaysState {
    clearTimeout(this.revertTimer);
    this.pending = null;
    if (this.previous) this.set(this.previous);
    this.previous = null;
    return this.changed();
  }

  private set(monitors: ProfileMonitor[]) {
    for (const monitor of this.monitors) {
      monitor.layout = clone(monitors.find((m) => m.identity.path === monitor.identity.path)?.layout ?? null);
    }
  }

  private capture(): ProfileMonitor[] {
    return this.monitors.map((monitor) => at(monitor, clone(monitor.layout)));
  }

  private matches(monitors: ProfileMonitor[]): boolean {
    return this.monitors.every((monitor) => {
      const saved = monitors.find((m) => m.identity.path === monitor.identity.path);
      return sameLayout(saved?.layout ?? null, monitor.layout);
    });
  }

  private suggestions(): DisplayProfile[] {
    let x = 0;
    const all = this.monitors.map((monitor, i) => {
      const value = layout(x, 0, monitor.layout?.mode ?? monitor.modes[0]!, i === 1);
      x += value.mode.width;
      return at(monitor, value);
    });
    const dx = all.find((monitor) => monitor.layout?.primary)!.layout!.x;
    for (const monitor of all) monitor.layout!.x -= dx;
    return [
      { id: 'all-monitors', name: 'All monitors', icon: 'monitors', monitors: all },
      ...this.monitors.map((alone) => ({
        id: slugify(alone.name),
        name: alone.name,
        icon: alone.built_in ? 'laptop' : 'monitor',
        monitors: this.monitors.map((monitor) => at(monitor, monitor === alone ? layout(0, 0, monitor.layout?.mode ?? monitor.modes[0]!, true) : null)),
      })),
    ];
  }

  private add(profile: DisplayProfile): DisplaysState {
    this.checkName(profile.name, null);
    let id = slugify(profile.name) || 'profile';
    for (let n = 2; this.profiles.some((p) => p.id === id); n += 1) id = `${slugify(profile.name)}-${n}`;
    this.profiles.push({ ...clone(profile), id, name: profile.name.trim() });
    return this.changed();
  }

  private checkName(name: string, id: string | null) {
    const trimmed = name.trim();
    if (!trimmed || trimmed.length > 40) throw new DaemonError('invalid_params', 'a profile name is 1 to 40 characters');
    if (this.profiles.some((p) => p.id !== id && p.name.toLowerCase() === trimmed.toLowerCase())) {
      throw new DaemonError('already_exists', `a profile named ${trimmed} already exists`);
    }
  }

  private index(id: string): number {
    const index = this.profiles.findIndex((profile) => profile.id === id);
    if (index < 0) throw new DaemonError('not_found', `no display profile ${id}`);
    return index;
  }

  private changed(): DisplaysState {
    const state = this.state;
    this.emit({ event: 'displays.changed', data: clone(state) });
    return state;
  }
}
