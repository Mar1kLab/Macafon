// Typed calls over a transport (Tauri bridge or the in-browser mock daemon).

import type {
  AbState,
  AbWaveform,
  AutostartState,
  CalibrationState,
  Device,
  DictationState,
  DisplayProfile,
  DisplaysState,
  DoctorReport,
  HotkeysState,
  Preferences,
  ErrorCode,
  Event,
  Hello,
  MicState,
  MixerState,
  PresetInfo,
  PlaybackDevice,
  Request,
  OutputControl,
  SoundpadState,
} from './protocol';

export type Method = Request['method'];

type ParamsOf<M extends Method> =
  Extract<Request, { method: M }> extends { params: infer P } ? P : undefined;

/** Result of every method; a new protocol method fails type-checking until listed here. */
interface Results {
  'output_controls.list': OutputControl[];
  'output_controls.set': OutputControl[];
  hello: Hello;
  'daemon.quit': null;
  'autostart.get': AutostartState;
  'autostart.set': AutostartState;
  'mic.get': MicState;
  'mic.set': MicState;
  'mic.set_preset': MicState;
  'mic.set_muted': MicState;
  'mic.toggle_mute': MicState;
  'mic.set_bypass': MicState;
  'mic.set_device': MicState;
  'mic.set_fallback_device': MicState;
  'mic.set_input_channel': MicState;
  'mic.set_default_source': null;
  'mic.restore_default_source': null;
  'presets.list': PresetInfo[];
  'presets.save': PresetInfo;
  'presets.update': PresetInfo;
  'presets.rename': PresetInfo;
  'presets.duplicate': PresetInfo;
  'presets.delete': null;
  'presets.export': unknown;
  'presets.import': PresetInfo;
  'devices.list': Device[];
  'devices.subscribe': null;
  'devices.unsubscribe': null;
  'meters.subscribe': null;
  'meters.unsubscribe': null;
  'calibration.start': CalibrationState;
  'calibration.cancel': CalibrationState;
  'calibration.get': CalibrationState;
  'ab.record': AbState;
  'ab.stop_recording': AbState;
  'ab.play': AbState;
  'ab.stop': AbState;
  'ab.select': AbState;
  'ab.toggle': AbState;
  'ab.seek': AbState;
  'ab.set_loudness_match': AbState;
  'ab.get': AbState;
  'ab.waveform': AbWaveform;
  'ab.outputs': PlaybackDevice[];
  'ab.export': null;
  'mixer.get': MixerState;
  'mixer.set_enabled': MixerState;
  'mixer.set_master': MixerState;
  'mixer.set_output': MixerState;
  'mixer.set_output_priority': MixerState;
  'mixer.set_auto_classify': MixerState;
  'channel.create': MixerState;
  'channel.update': MixerState;
  'channel.delete': MixerState;
  'channel.set': MixerState;
  'channel.adjust_volume': MixerState;
  'channel.toggle_mute': MixerState;
  'channel.set_default': MixerState;
  'channel.reorder': MixerState;
  'apps.move': MixerState;
  'apps.set_managed': MixerState;
  'apps.forget': MixerState;
  'apps.set_volume': MixerState;
  'apps.icon': string | null;
  'doctor.run': DoctorReport;
  'doctor.fix': null;
  'mic.cycle_preset': MicState;
  'hotkeys.get': HotkeysState;
  'hotkeys.set': HotkeysState;
  'ui.window_closed': null;
  'prefs.get': Preferences;
  'prefs.set': Preferences;
  'soundpad.get': SoundpadState;
  'soundpad.rescan': SoundpadState;
  'soundpad.open_folder': null;
  'soundpad.set_folder': SoundpadState;
  'soundpad.play': null;
  'soundpad.play_random': null;
  'soundpad.stop': null;
  'soundpad.set_paused': null;
  'soundpad.seek': null;
  'soundpad.set_looping': null;
  'soundpad.set_prefs': SoundpadState;
  'soundpad.set_monitor_device': SoundpadState;
  'soundpad.set_sound': SoundpadState;
  'soundpad.rename': SoundpadState;
  'soundpad.move': SoundpadState;
  'soundpad.delete': SoundpadState;
  'soundpad.delete_problem_sounds': SoundpadState;
  'soundpad.create_category': SoundpadState;
  'soundpad.rename_category': SoundpadState;
  'soundpad.delete_category': SoundpadState;
  /** The state once `done`, `null` for intermediate chunks. */
  'soundpad.upload': SoundpadState | null;
  'dictation.get': DictationState;
  'dictation.set_prefs': DictationState;
  'dictation.download': DictationState;
  'dictation.cancel_download': DictationState;
  'dictation.delete_model': DictationState;
  'dictation.import_model': DictationState;
  'dictation.set_models_dir': DictationState;
  'dictation.cancel_move': DictationState;
  'dictation.test_start': DictationState;
  'dictation.test_stop': DictationState;
  'dictation.cancel': DictationState;
  'dictation.history_clear': DictationState;
  'dictation.reinsert': null;
  'displays.get': DisplaysState;
  'displays.save': DisplaysState;
  'displays.create': DisplaysState;
  'displays.suggestions': DisplayProfile[];
  'displays.update': DisplaysState;
  'displays.recapture': DisplaysState;
  'displays.delete': DisplaysState;
  'displays.reorder': DisplaysState;
  'displays.apply': DisplaysState;
  'displays.confirm': DisplaysState;
  'displays.revert': DisplaysState;
  'displays.all_on': DisplaysState;
}

type ResultOf<M extends Method> = M extends keyof Results ? Results[M] : never;
// Fails to compile when Results misses a method of the generated protocol.
const _complete: Record<Method, true> = {} as Record<keyof Results, true>;
void _complete;

export type DaemonStatus =
  | { state: 'connecting' }
  | { state: 'connected'; version: string }
  | { state: 'unavailable'; reason: string };

/** Daemon error codes plus the bridge's own (daemon gone, no reply). */
export type ClientErrorCode = ErrorCode | 'unavailable' | 'timeout';

export class DaemonError extends Error {
  constructor(
    readonly code: ClientErrorCode,
    message: string,
  ) {
    super(message);
  }
}

export interface Transport {
  call(method: string, params: unknown): Promise<unknown>;
  onEvent(handler: (event: Event) => void): () => void;
  onStatus(handler: (status: DaemonStatus) => void): () => void;
  /** Try to reach (and start) the daemon again after it was unavailable. */
  retry(): void;
}

export class DaemonClient {
  constructor(readonly transport: Transport) {}

  call<M extends Method>(
    method: M,
    ...params: ParamsOf<M> extends undefined ? [] : [ParamsOf<M>]
  ): Promise<ResultOf<M>> {
    return this.transport.call(method, params[0] ?? null) as Promise<ResultOf<M>>;
  }
}
