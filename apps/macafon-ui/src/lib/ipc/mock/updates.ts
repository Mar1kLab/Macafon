// Updates of the mock daemon (PRD §8.6): checks, downloads with progress and tool installs,
// all simulated in memory. `?updates=available` starts as if a check found new versions,
// `?updates=error` makes the next download fail its checksum.
import { DaemonError } from '../client';
import type { ComponentUpdate, Event, UpdateComponent, UpdatesState } from '../protocol';
import { clone } from './helpers';

const LATEST_APP = '0.14.0';
const LATEST_YTDLP = '2026.10.29';
const LATEST_FFMPEG = 'autobuild-2026-10-31-14-02';
const MB = 1024 * 1024;

/** What the SoundDeck downloader sees after a tool install or removal. */
export interface ToolsSeen {
  ytdlp: { path: string; version: string } | null;
  ffmpeg: boolean;
}

function component(name: UpdateComponent, extra: Partial<ComponentUpdate>): ComponentUpdate {
  return {
    component: name,
    installable: true,
    source: 'missing',
    path: null,
    installed_version: null,
    latest_version: null,
    update_available: false,
    release_url: null,
    download_bytes: null,
    stage: 'idle',
    progress_pct: null,
    error: null,
    ...extra,
  };
}

export class MockUpdates {
  readonly state: UpdatesState;
  private timers = new Map<UpdateComponent, number>();
  private failNext: boolean;

  constructor(
    private emit: (event: Event) => void,
    private windows: boolean,
    appVersion: string,
    ytdlpOnPath: boolean,
    private onTools: (seen: ToolsSeen) => void,
  ) {
    const query = new URLSearchParams(globalThis.location?.search);
    this.failNext = query.get('updates') === 'error';
    this.state = {
      check_automatically: false,
      last_checked_unix: null,
      app: component('app', { installable: windows, source: 'bundled', installed_version: appVersion }),
      tools: [
        component('yt_dlp', {
          installable: windows,
          source: ytdlpOnPath ? 'path' : 'missing',
          path: ytdlpOnPath ? (windows ? 'C:\\Users\\Alex\\AppData\\Local\\Microsoft\\WinGet\\Links\\yt-dlp.exe' : '/usr/bin/yt-dlp') : null,
          installed_version: ytdlpOnPath ? '2026.10.22' : null,
        }),
        component('ffmpeg', {
          installable: windows,
          source: ytdlpOnPath ? 'path' : 'missing',
          path: ytdlpOnPath ? (windows ? 'C:\\Users\\Alex\\AppData\\Local\\Microsoft\\WinGet\\Links\\ffmpeg.exe' : '/usr/bin/ffmpeg') : null,
          installed_version: ytdlpOnPath ? '7.1' : null,
        }),
      ],
    };
    if (query.get('updates') === 'available') this.found();
  }

  call(method: string, p: Record<string, unknown>): unknown {
    switch (method) {
      case 'updates.get':
        return clone(this.state);
      case 'updates.check':
        this.check();
        return null;
      case 'updates.install':
        this.install(p.component as UpdateComponent);
        return null;
      case 'updates.cancel':
        this.stop(p.component as UpdateComponent);
        this.patch(p.component as UpdateComponent, { stage: 'idle', progress_pct: null });
        return null;
      case 'updates.remove':
        this.remove(p.component as UpdateComponent);
        return null;
      case 'updates.set_check_automatically':
        this.state.check_automatically = p.enabled as boolean;
        this.changed();
        return null;
      case 'updates.open_release':
        return null;
      default:
        throw new DaemonError('invalid_params', `unknown method ${method}`);
    }
  }

  private toolsDir(): string {
    return this.windows ? 'C:\\Users\\Alex\\AppData\\Local\\Macafon\\tools' : '';
  }

  private all(): ComponentUpdate[] {
    return [this.state.app, ...this.state.tools];
  }

  private get(name: UpdateComponent): ComponentUpdate {
    const found = this.all().find((entry) => entry.component === name);
    if (!found) throw new DaemonError('not_found', `unknown component ${name}`);
    return found;
  }

  private patch(name: UpdateComponent, patch: Partial<ComponentUpdate>) {
    Object.assign(this.get(name), patch);
    this.changed();
  }

  private check() {
    if (this.all().some((entry) => entry.stage === 'checking')) return;
    for (const entry of this.all()) {
      if (entry.stage === 'idle' || entry.stage === 'failed') Object.assign(entry, { stage: 'checking', error: null });
    }
    this.changed();
    setTimeout(() => {
      this.found();
      for (const entry of this.all()) if (entry.stage === 'checking') entry.stage = 'idle';
      this.changed();
    }, 900);
  }

  /** The versions a check finds on GitHub. */
  private found() {
    const latest: Record<UpdateComponent, [string, number, string]> = {
      app: [LATEST_APP, 28 * MB, `https://github.com/Mar1kLab/Macafon/releases/tag/v${LATEST_APP}`],
      yt_dlp: [LATEST_YTDLP, 18 * MB, `https://github.com/yt-dlp/yt-dlp/releases/tag/${LATEST_YTDLP}`],
      ffmpeg: [LATEST_FFMPEG, 142 * MB, `https://github.com/yt-dlp/FFmpeg-Builds/releases/tag/latest`],
    };
    for (const entry of this.all()) {
      const [version, bytes, url] = latest[entry.component];
      entry.latest_version = version;
      entry.download_bytes = bytes;
      entry.release_url = url;
      entry.update_available = this.outdated(entry);
    }
    this.state.last_checked_unix = Math.floor(Date.now() / 1000);
  }

  private outdated(entry: ComponentUpdate): boolean {
    if (!entry.latest_version || entry.installed_version === entry.latest_version) return false;
    // The app compares versions; a tool installed elsewhere is not Macafon's to update.
    return entry.component === 'app' || entry.source === 'managed';
  }

  private install(name: UpdateComponent) {
    const entry = this.get(name);
    if (!entry.installable) throw new DaemonError('invalid_params','install it with the package manager');
    if (name === 'app' && entry.stage === 'ready') {
      this.patch(name, { stage: 'installing', progress_pct: null });
      setTimeout(() => this.patch(name, { stage: 'ready' }), 2500);
      return;
    }
    if (['downloading', 'verifying', 'installing'].includes(entry.stage)) return;
    if (!entry.latest_version) this.found();
    this.patch(name, { stage: 'downloading', progress_pct: 0, error: null });
    const speedPct = name === 'ffmpeg' ? 3 : 7;
    const tick = () => {
      const current = this.get(name);
      const progress = Math.min(100, (current.progress_pct ?? 0) + speedPct);
      if (progress < 100) {
        this.patch(name, { progress_pct: progress });
        this.timers.set(name, window.setTimeout(tick, 120));
        return;
      }
      this.patch(name, { stage: 'verifying', progress_pct: null });
      this.timers.set(name, window.setTimeout(() => this.verified(name), 600));
    };
    this.timers.set(name, window.setTimeout(tick, 120));
  }

  private verified(name: UpdateComponent) {
    this.timers.delete(name);
    if (this.failNext) {
      this.failNext = false;
      this.patch(name, { stage: 'failed', error: 'Checksum mismatch: the download was deleted' });
      return;
    }
    if (name === 'app') {
      this.patch(name, { stage: 'ready' });
      return;
    }
    this.patch(name, { stage: 'installing' });
    this.timers.set(
      name,
      window.setTimeout(() => {
        this.timers.delete(name);
        const entry = this.get(name);
        const file = name === 'yt_dlp' ? 'yt-dlp.exe' : 'ffmpeg.exe';
        Object.assign(entry, {
          stage: 'idle',
          source: 'managed',
          path: `${this.toolsDir()}\\${file}`,
          installed_version: entry.latest_version,
          update_available: false,
        });
        this.changed();
        this.toolsChanged();
      }, 500),
    );
  }

  private remove(name: UpdateComponent) {
    const entry = this.get(name);
    if (entry.source !== 'managed') throw new DaemonError('invalid_params', 'only Macafon’s own copy can be removed');
    Object.assign(entry, { source: 'missing', path: null, installed_version: null, update_available: false, error: null });
    this.changed();
    this.toolsChanged();
  }

  private stop(name: UpdateComponent) {
    const timer = this.timers.get(name);
    if (timer !== undefined) clearTimeout(timer);
    this.timers.delete(name);
  }

  private toolsChanged() {
    const [ytdlp, ffmpeg] = this.state.tools;
    this.onTools({
      ytdlp: ytdlp?.path && ytdlp.installed_version ? { path: ytdlp.path, version: ytdlp.installed_version } : null,
      ffmpeg: !!ffmpeg?.path,
    });
  }

  private changed() {
    this.emit({ event: 'updates.changed', data: clone(this.state) });
  }
}
