// Dictation of the mock daemon (PRD §8.4): the model manifest with downloads, the worker's
// load/unload cycle, live levels while listening and canned transcripts, so every state of the
// Dictation screen can be seen in a browser. `?dictation=ready` starts with Turbo downloaded and
// the feature on, `?dictation=downloading` with a download running, `?dictation=error` with a
// crashed worker.
import { DaemonError } from '../client';
import type {
  DictationLanguage,
  DictationModel,
  DictationPrefs,
  DictationPrefsPatch,
  DictationResult,
  DictationState,
  Event,
} from '../protocol';
import { clone } from './helpers';
import { voiceLevel } from './signals';

const MB = 1024 * 1024;
const DEFAULT_FOLDER = 'C:\\Users\\you\\AppData\\Local\\Macafon\\models\\stt';
const DOWNLOAD_BYTES_PER_SEC = 24 * MB;
const LOGICAL_CORES = 12;
const DEFAULT_THREADS = 3;

const SAMPLES: Record<DictationLanguage, string[]> = {
  en: ['Thanks, I will send the build tonight.', 'Can we move the call to four thirty?', 'Ship it once the tests are green.'],
  uk: ['Привіт, це перевірка диктовки.', 'Скинь, будь ласка, посилання на реліз.', 'Зустрінемось о пів на п’яту.'],
  ru: ['Привет, это проверка диктовки.', 'Давай созвонимся после обеда.', 'Сборку выложу сегодня вечером.'],
};

function manifest(): DictationModel[] {
  const model = (id: string, engine: DictationModel['engine'], name: string, size_mb: number, quality: number, speed: number, recommended = false): DictationModel => ({
    id, engine, name, size_bytes: Math.round(size_mb * MB), quality, speed, recommended,
    status: 'missing', downloaded_bytes: 0, bytes_per_sec: null, error: null,
  });
  return [
    model('whisper-large-v3-turbo-q5_0', 'whisper', 'Turbo', 548, 5, 2, true),
    model('whisper-small-q5_1', 'whisper', 'Small', 182, 2, 4),
    model('parakeet-tdt-0.6b-v3-int8', 'parakeet', 'Parakeet v3', 642, 4, 5),
  ];
}

export class MockDictation {
  state: DictationState;
  private downloads = new Map<string, number>();
  private moveTimer: number | undefined;
  private unloadTimer: number | undefined;
  private listenTimer: number | undefined;
  private listenStarted = 0;
  private nextId = 1;
  private sample = 0;

  constructor(
    private readonly emit: (event: Event) => void,
    mode: string | null,
  ) {
    const prefs: DictationPrefs = {
      enabled: false,
      engine: 'whisper',
      model: null,
      language: 'layout',
      fixed_language: 'uk',
      tap: 'clean',
      mute_mic_while_dictating: true,
      insert: 'paste',
      trailing_space: true,
      fullscreen_games: 'copy',
      unload_after_s: 180,
      threads: 0,
      game_mode: true,
      gpu: true,
      pre_roll: true,
      keep_history: true,
      save_history: false,
      vocabulary: ['Macafon', 'PipeWire'],
      popup_look: 'flow',
      popup_position: 'bottom',
      popup_offset_pct: 0,
      popup_primary_monitor: false,
      show_in_overlay: true,
    };
    this.state = {
      prefs,
      phase: 'off',
      error: null,
      loaded_model: null,
      worker_ram_bytes: null,
      worker_threads: null,
      default_threads: DEFAULT_THREADS,
      logical_cores: LOGICAL_CORES,
      models: manifest(),
      storage: { folder: DEFAULT_FOLDER, default_folder: true, free_bytes: 212 * 1024 * MB, used_bytes: 0, rotational: false, moving: null },
      history: [],
    };
    if (mode === 'ready' || mode === 'error') {
      this.finishDownload('whisper-large-v3-turbo-q5_0');
      prefs.enabled = true;
      prefs.model = 'whisper-large-v3-turbo-q5_0';
      this.state.phase = 'unloaded';
      this.state.history = [
        this.result('Thanks, I will send the build tonight.', 'en', 2600, 'inserted', 5 * 60_000),
        this.result('Привіт, це перевірка диктовки.', 'uk', 3100, 'inserted', 22 * 60_000),
        this.result('Давай созвонимся после обеда.', 'ru', 2200, 'copied', 61 * 60_000),
      ];
      if (mode === 'error') {
        this.state.phase = 'error';
        this.state.error = 'The speech worker stopped unexpectedly';
      }
    }
    if (mode === 'downloading') this.download('whisper-large-v3-turbo-q5_0', 0.42);
  }

  private changed(): DictationState {
    this.emit({ event: 'dictation.changed', data: clone(this.state) });
    return clone(this.state);
  }

  private model(id: string): DictationModel {
    const model = this.state.models.find((m) => m.id === id);
    if (!model) throw new DaemonError('not_found', `no model ${id}`);
    return model;
  }

  private usedBytes(): number {
    return this.state.models.filter((m) => m.status === 'ready').reduce((sum, m) => sum + m.size_bytes, 0);
  }

  private finishDownload(id: string) {
    const model = this.model(id);
    model.status = 'ready';
    model.downloaded_bytes = model.size_bytes;
    model.bytes_per_sec = null;
    this.state.storage.used_bytes = this.usedBytes();
    this.state.storage.free_bytes = (this.state.storage.free_bytes ?? 0) - model.size_bytes;
    if (!this.state.prefs.model || this.model(this.state.prefs.model).engine !== this.state.prefs.engine) {
      if (model.engine === this.state.prefs.engine) this.state.prefs.model = id;
    }
  }

  private download(id: string, from = 0) {
    const model = this.model(id);
    if (model.status === 'ready' || this.downloads.has(id)) return;
    model.status = 'downloading';
    model.error = null;
    model.downloaded_bytes = Math.round(model.size_bytes * from);
    model.bytes_per_sec = DOWNLOAD_BYTES_PER_SEC;
    const timer = window.setInterval(() => {
      // Downloads report about 4 times a second (§8.4.6); speed wanders a little.
      model.bytes_per_sec = Math.round(DOWNLOAD_BYTES_PER_SEC * (0.8 + Math.random() * 0.4));
      model.downloaded_bytes = Math.min(model.size_bytes, model.downloaded_bytes + model.bytes_per_sec / 4);
      if (model.downloaded_bytes >= model.size_bytes) {
        clearInterval(timer);
        this.downloads.delete(id);
        this.finishDownload(id);
      }
      this.changed();
    }, 250);
    this.downloads.set(id, timer);
  }

  private result(text: string, language: DictationLanguage | null, audio_ms: number, outcome: DictationResult['outcome'], ago = 0): DictationResult {
    const model = this.state.prefs.model ?? 'whisper-large-v3-turbo-q5_0';
    return {
      id: this.nextId++,
      text,
      language,
      engine: this.state.prefs.engine,
      model,
      audio_ms,
      elapsed_ms: this.state.prefs.engine === 'parakeet' ? 380 + Math.round(audio_ms / 20) : 900 + Math.round(audio_ms / 6),
      at_unix_ms: Date.now() - ago,
      outcome,
    };
  }

  private scheduleUnload() {
    clearTimeout(this.unloadTimer);
    const after = this.state.prefs.unload_after_s;
    if (after < 0) return;
    this.unloadTimer = window.setTimeout(() => {
      if (this.state.phase !== 'ready') return;
      this.state.phase = 'unloaded';
      this.state.loaded_model = null;
      this.state.worker_ram_bytes = null;
      this.state.worker_threads = null;
      this.changed();
    }, Math.max(after, 3) * 1000);
  }

  private startListening() {
    const prefs = this.state.prefs;
    if (!prefs.enabled) throw new DaemonError('invalid_params', 'dictation is off');
    if (!prefs.model || this.model(prefs.model).status !== 'ready') throw new DaemonError('invalid_params', 'download a model first');
    if (this.state.phase === 'listening' || this.state.phase === 'transcribing') throw new DaemonError('busy', 'already dictating');
    clearTimeout(this.unloadTimer);
    const cold = this.state.phase !== 'ready';
    this.state.phase = 'listening';
    this.state.error = null;
    this.listenStarted = performance.now();
    if (cold) {
      // The worker loads behind the speech (STT-007).
      window.setTimeout(() => {
        this.state.loaded_model = prefs.model;
        this.state.worker_ram_bytes = (prefs.engine === 'parakeet' ? 980 : prefs.model?.includes('small') ? 410 : 1130) * MB;
        this.state.worker_threads = prefs.threads || DEFAULT_THREADS;
        if (this.state.phase === 'listening') this.changed();
      }, 900);
    }
    clearInterval(this.listenTimer);
    this.listenTimer = window.setInterval(() => {
      const t = (performance.now() - this.listenStarted) / 1000;
      const rms = voiceLevel(t + 0.7, -58) + 22;
      this.emit({ event: 'dictation.level', data: { rms_dbfs: rms, peak_dbfs: rms + 9 } });
    }, 1000 / 30);
    return this.changed();
  }

  private stopListening(outcome: DictationResult['outcome']) {
    if (this.state.phase !== 'listening') throw new DaemonError('invalid_params', 'not dictating');
    clearInterval(this.listenTimer);
    const audio_ms = Math.round(performance.now() - this.listenStarted);
    this.state.phase = 'transcribing';
    const prefs = this.state.prefs;
    const language: DictationLanguage = prefs.language === 'fixed' ? prefs.fixed_language : (['uk', 'en', 'ru'] as const)[this.sample % 3]!;
    const result = this.result(SAMPLES[language][this.sample++ % 3]!, prefs.engine === 'parakeet' ? null : language, audio_ms, outcome);
    window.setTimeout(() => {
      this.state.phase = 'ready';
      this.state.loaded_model = prefs.model;
      this.state.worker_ram_bytes ??= 1130 * MB;
      this.state.worker_threads ??= prefs.threads || DEFAULT_THREADS;
      if (audio_ms < 300) {
        this.changed();
        this.scheduleUnload();
        return;
      }
      if (outcome !== 'test' && prefs.keep_history) this.state.history = [result, ...this.state.history].slice(0, 20);
      this.emit({ event: 'dictation.result', data: clone(result) });
      this.changed();
      this.scheduleUnload();
    }, result.elapsed_ms);
    return this.changed();
  }

  call(method: string, p: Record<string, unknown>): unknown {
    const prefs = this.state.prefs;
    switch (method) {
      case 'dictation.get':
        return clone(this.state);
      case 'dictation.set_prefs': {
        const patch = p.patch as DictationPrefsPatch;
        if (patch.enabled && !this.state.models.some((m) => m.status === 'ready')) {
          throw new DaemonError('invalid_params', 'download a model first');
        }
        for (const [key, value] of Object.entries(patch)) {
          if (value !== undefined && value !== null) (prefs as Record<string, unknown>)[key] = clone(value);
        }
        if (patch.engine) {
          // Switching engines unloads the worker and picks a downloaded model of that engine.
          const current = prefs.model ? this.model(prefs.model) : null;
          if (current?.engine !== patch.engine) {
            prefs.model = this.state.models.find((m) => m.engine === patch.engine && m.status === 'ready')?.id ?? null;
          }
          if (this.state.phase === 'ready') this.state.phase = 'unloaded';
          this.state.loaded_model = null;
          this.state.worker_ram_bytes = null;
          this.state.worker_threads = null;
        }
        if (patch.enabled !== undefined && patch.enabled !== null) {
          this.state.phase = patch.enabled ? 'unloaded' : 'off';
          this.state.error = null;
          if (!patch.enabled) {
            this.state.loaded_model = null;
            this.state.worker_ram_bytes = null;
            this.state.worker_threads = null;
          }
        }
        if (patch.keep_history === false) this.state.history = [];
        return this.changed();
      }
      case 'dictation.download':
        this.download(String(p.model));
        return this.changed();
      case 'dictation.cancel_download': {
        const id = String(p.model);
        clearInterval(this.downloads.get(id));
        this.downloads.delete(id);
        const model = this.model(id);
        model.status = 'missing';
        model.bytes_per_sec = null;
        return this.changed();
      }
      case 'dictation.delete_model': {
        const model = this.model(String(p.model));
        model.status = 'missing';
        model.downloaded_bytes = 0;
        if (this.state.loaded_model === model.id) {
          this.state.loaded_model = null;
          this.state.worker_ram_bytes = null;
          this.state.phase = prefs.enabled ? 'unloaded' : 'off';
        }
        if (prefs.model === model.id) prefs.model = this.state.models.find((m) => m.engine === prefs.engine && m.status === 'ready')?.id ?? null;
        this.state.storage.free_bytes = (this.state.storage.free_bytes ?? 0) + model.size_bytes;
        this.state.storage.used_bytes = this.usedBytes();
        if (!this.state.models.some((m) => m.status === 'ready')) {
          prefs.enabled = false;
          this.state.phase = 'off';
        }
        return this.changed();
      }
      case 'dictation.import_model':
        throw new DaemonError('invalid_params', 'the file does not match any model of the manifest');
      case 'dictation.set_models_dir': {
        const folder = (p.folder as string | null) ?? DEFAULT_FOLDER;
        const total = this.usedBytes();
        const storage = this.state.storage;
        if (total === 0) {
          storage.folder = folder;
          storage.default_folder = folder === DEFAULT_FOLDER;
          storage.rotational = /^D:/i.test(folder);
          return this.changed();
        }
        storage.moving = { to: folder, done_bytes: 0, total_bytes: total };
        clearInterval(this.moveTimer);
        this.moveTimer = window.setInterval(() => {
          const moving = storage.moving;
          if (!moving) return clearInterval(this.moveTimer);
          moving.done_bytes = Math.min(moving.total_bytes, moving.done_bytes + 60 * MB);
          if (moving.done_bytes >= moving.total_bytes) {
            clearInterval(this.moveTimer);
            storage.folder = moving.to;
            storage.default_folder = moving.to === DEFAULT_FOLDER;
            storage.rotational = /^D:/i.test(moving.to);
            storage.moving = null;
          }
          this.changed();
        }, 250);
        return this.changed();
      }
      case 'dictation.cancel_move':
        clearInterval(this.moveTimer);
        this.state.storage.moving = null;
        return this.changed();
      case 'dictation.test_start':
        return this.startListening();
      case 'dictation.test_stop':
        return this.stopListening('test');
      case 'dictation.cancel':
        clearInterval(this.listenTimer);
        if (this.state.phase === 'listening') this.state.phase = this.state.loaded_model ? 'ready' : 'unloaded';
        return this.changed();
      case 'dictation.history_clear':
        this.state.history = [];
        return this.changed();
      case 'dictation.reinsert':
        if (!this.state.history.some((r) => r.id === Number(p.id))) throw new DaemonError('not_found', 'no such result');
        return null;
      default:
        throw new DaemonError('bad_request', `unknown method ${method}`);
    }
  }
}
