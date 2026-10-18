// Soundpad state and requests of the mock daemon: a plausible library, playback progress and
// uploads kept in memory.
import { DaemonError } from '../client';
import type { Event, SoundInfo, SoundpadPlayback, SoundpadState } from '../protocol';
import { clone } from './helpers';

const EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'm4a', 'aac'];

let initialIndex = 0;

function sound(id: string, duration_ms: number, extra: Partial<SoundInfo> = {}): SoundInfo {
  const file = id.split('/').pop()!;
  const category = id.includes('/') ? id.split('/')[0]! : null;
  return {
    id,
    name: file.replace(/\.[^.]+$/, ''),
    category,
    duration_ms,
    volume_pct: 100,
    favorite: false,
    play_count: 0,
    added_unix_ms: Date.now() - (initialIndex++ % 16) * 86_400_000,
    error: null,
    ...extra,
  };
}

function initialSounds(): SoundInfo[] {
  return [
    sound('Air horn.mp3', 2400, { favorite: true, play_count: 41 }),
    sound('Drum roll.wav', 4100, { play_count: 7 }),
    sound('Sad trombone.mp3', 3800, { play_count: 18 }),
    sound('Memes/Bruh.mp3', 900, { favorite: true, play_count: 66 }),
    sound('Memes/Emotional damage.mp3', 1700, { play_count: 23 }),
    sound('Memes/Oh no no no.ogg', 5200, { play_count: 4 }),
    sound('Memes/Vine boom.wav', 1100, { volume_pct: 70, play_count: 88 }),
    sound('Memes/Windows XP error.wav', 800),
    sound('Music/Victory fanfare.flac', 7600, { play_count: 3 }),
    sound('Music/Elevator loop.mp3', 31_000),
    sound('Music/Suspense sting.m4a', 2900),
    sound('Reactions/Applause.mp3', 6200, { play_count: 12 }),
    sound('Reactions/Crickets.ogg', 4800, { play_count: 9 }),
    sound('Reactions/Laugh track.mp3', 3300, { play_count: 15 }),
    sound('Reactions/Wow.mp3', 1300),
    sound('Reactions/Broken.mp3', 0, { duration_ms: null, error: 'Unsupported codec' }),
  ];
}

function defaultFolder(windows: boolean): string {
  return windows ? 'C:\\Users\\Alex\\Music\\Macafon Sounds' : '/home/alex/Music/Macafon Sounds';
}

export class MockSoundpad {
  readonly state: SoundpadState;
  private timer: number | undefined;
  private startedAt = 0;
  private offsetMs = 0;
  private looping = false;
  private uploads = new Map<string, number>();

  constructor(private emit: (event: Event) => void, private windows: boolean) {
    const query = new URLSearchParams(globalThis.location?.search);
    this.state = {
      folder: defaultFolder(windows),
      default_folder: true,
      categories: ['Memes', 'Music', 'Reactions'],
      sounds: initialSounds(),
      prefs: {
        to_microphone: true,
        to_headphones: true,
        microphone_volume_pct: 100,
        headphones_volume_pct: 70,
        mute_voice_while_playing: false,
        monitor_device: null,
        show_in_overlay: true,
        overlay_view: query.get('sounds') === 'list' ? 'list' : 'grid',
        overlay_controls: query.get('controls') === 'bottom' ? 'bottom' : query.get('controls') === 'hidden' ? 'hidden' : 'top',
      },
      playback: null,
      microphone_ready: true,
    };
  }

  call(method: string, p: Record<string, unknown>): unknown {
    const s = this.state;
    switch (method) {
      case 'soundpad.get':
      case 'soundpad.rescan':
        return clone(s);
      case 'soundpad.open_folder':
        return null;
      case 'soundpad.set_folder': {
        const folder = ((p.folder as string | null) ?? '').trim();
        const absolute = this.windows ? /^[a-zA-Z]:[\\/]|^\\\\/.test(folder) : folder.startsWith('/');
        if (folder && !absolute) throw new DaemonError('invalid_params', `${folder} is not a full path`);
        this.stop();
        s.folder = folder || defaultFolder(this.windows);
        s.default_folder = !folder || folder === defaultFolder(this.windows);
        return this.changed();
      }
      case 'soundpad.play':
        this.play(this.sound(String(p.id)));
        return null;
      case 'soundpad.play_random': {
        const category = (p.category as string | null) ?? null;
        const pool = s.sounds.filter((x) => !x.error && (category === null || x.category === category));
        if (pool.length === 0) throw new DaemonError('not_found', 'no sounds to play');
        this.play(pool[Math.floor(Math.random() * pool.length)]!);
        return null;
      }
      case 'soundpad.stop':
        this.stop();
        return null;
      case 'soundpad.set_paused': {
        const playback = s.playback;
        if (!playback) return null;
        const paused = (p.paused as boolean | null) ?? !playback.paused;
        if (paused !== playback.paused) {
          this.offsetMs = this.position();
          this.startedAt = performance.now();
          playback.paused = paused;
        }
        this.progress();
        return null;
      }
      case 'soundpad.seek':
        if (s.playback) {
          this.offsetMs = Math.min(Number(p.position_ms), s.playback.duration_ms);
          this.startedAt = performance.now();
          this.progress();
        }
        return null;
      case 'soundpad.set_looping':
        this.looping = Boolean(p.looping);
        if (s.playback) s.playback.looping = this.looping;
        this.progress();
        return null;
      case 'soundpad.set_prefs':
        Object.assign(s.prefs, p.patch);
        return this.changed();
      case 'soundpad.set_monitor_device':
        s.prefs.monitor_device = (p.device as string | null) ?? null;
        return this.changed();
      case 'soundpad.set_sound':
        Object.assign(this.sound(String(p.id)), p.patch);
        return this.changed();
      case 'soundpad.rename': {
        const target = this.sound(String(p.id));
        const name = String(p.name).trim();
        if (!name || /[\\/:*?"<>|]/.test(name)) throw new DaemonError('invalid_params', 'invalid file name');
        const extension = target.id.split('.').pop();
        this.rekey(target, `${target.category ? `${target.category}/` : ''}${name}.${extension}`);
        return this.changed();
      }
      case 'soundpad.move': {
        const target = this.sound(String(p.id));
        const category = (p.category as string | null) ?? null;
        const file = target.id.split('/').pop()!;
        this.rekey(target, `${category ? `${category}/` : ''}${file}`);
        return this.changed();
      }
      case 'soundpad.delete':
        if (s.playback?.sound_id === p.id) this.stop();
        s.sounds = s.sounds.filter((x) => x.id !== p.id);
        return this.changed();
      case 'soundpad.create_category': {
        const name = String(p.name).trim();
        if (s.categories.some((c) => c.toLowerCase() === name.toLowerCase())) throw new DaemonError('already_exists', 'category exists');
        s.categories = [...s.categories, name].sort((a, b) => a.localeCompare(b));
        return this.changed();
      }
      case 'soundpad.rename_category': {
        const from = String(p.from);
        const to = String(p.to).trim();
        s.categories = s.categories.map((c) => (c === from ? to : c)).sort((a, b) => a.localeCompare(b));
        for (const x of s.sounds.filter((x) => x.category === from)) this.rekey(x, `${to}/${x.id.split('/').pop()}`);
        return this.changed();
      }
      case 'soundpad.delete_category': {
        const name = String(p.name);
        s.categories = s.categories.filter((c) => c !== name);
        for (const x of s.sounds.filter((x) => x.category === name)) this.rekey(x, x.id.split('/').pop()!);
        return this.changed();
      }
      case 'soundpad.upload': {
        const fileName = String(p.file_name);
        const extension = fileName.split('.').pop()?.toLowerCase() ?? '';
        if (!EXTENSIONS.includes(extension)) throw new DaemonError('invalid_params', `unsupported file type .${extension}`);
        const uploadId = String(p.upload_id);
        const received = (this.uploads.get(uploadId) ?? 0) + atob(String(p.data)).length;
        this.uploads.set(uploadId, received);
        if (!p.done) return null;
        this.uploads.delete(uploadId);
        const category = (p.category as string | null) ?? null;
        let id = `${category ? `${category}/` : ''}${fileName}`;
        for (let n = 2; s.sounds.some((x) => x.id === id); n++) {
          id = `${category ? `${category}/` : ''}${fileName.replace(/(\.[^.]+)$/, ` (${n})$1`)}`;
        }
        // Rough MP3 duration at 128 kbit/s.
        s.sounds.push(sound(id, Math.max(500, Math.round(received / 16)), { added_unix_ms: Date.now() }));
        this.sort();
        return this.changed();
      }
      default:
        throw new DaemonError('bad_request', `unknown method ${method}`);
    }
  }

  private sound(id: string): SoundInfo {
    const found = this.state.sounds.find((x) => x.id === id);
    if (!found) throw new DaemonError('not_found', `unknown sound ${id}`);
    return found;
  }

  private rekey(target: SoundInfo, id: string) {
    if (this.state.sounds.some((x) => x !== target && x.id === id)) throw new DaemonError('already_exists', 'a sound with this name exists');
    if (this.state.playback?.sound_id === target.id) this.state.playback.sound_id = id;
    target.id = id;
    target.name = id.split('/').pop()!.replace(/\.[^.]+$/, '');
    target.category = id.includes('/') ? id.split('/')[0]! : null;
    this.sort();
  }

  private sort() {
    this.state.sounds.sort(
      (a, b) => (a.category ?? '').localeCompare(b.category ?? '') || a.name.localeCompare(b.name),
    );
  }

  private play(target: SoundInfo) {
    if (target.error || target.duration_ms === null) throw new DaemonError('internal', target.error ?? 'cannot decode');
    target.play_count += 1;
    this.state.playback = {
      sound_id: target.id,
      position_ms: 0,
      duration_ms: target.duration_ms,
      paused: false,
      looping: this.looping,
      loading: false,
    };
    this.startedAt = performance.now();
    this.offsetMs = 0;
    clearInterval(this.timer);
    this.timer = window.setInterval(() => this.progress(), 100);
    this.changed();
    this.progress();
  }

  private stop() {
    clearInterval(this.timer);
    this.state.playback = null;
    this.emit({ event: 'soundpad.playback', data: null });
  }

  private position(): number {
    const playback = this.state.playback;
    if (!playback) return 0;
    return playback.paused ? this.offsetMs : this.offsetMs + performance.now() - this.startedAt;
  }

  private progress() {
    const playback = this.state.playback;
    if (!playback) return;
    let position = this.position();
    if (position >= playback.duration_ms) {
      if (!playback.looping) return this.stop();
      this.offsetMs = 0;
      this.startedAt = performance.now();
      position = 0;
    }
    playback.position_ms = Math.round(position);
    this.emit({ event: 'soundpad.playback', data: clone(playback) as SoundpadPlayback });
  }

  private changed(): SoundpadState {
    this.emit({ event: 'soundpad.changed', data: clone(this.state) });
    return clone(this.state);
  }
}
