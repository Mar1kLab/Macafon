// UI state mirrored from the daemon. The UI only talks IPC (AGENTS.md §5): every change is a
// request, and the daemon's reply or event is the truth.

import { DaemonClient, DaemonError, type DaemonStatus, type Transport } from '../ipc/client';
import type {
  AbState,
  AutostartState,
  CalibrationState,
  ChainSettings,
  Device,
  Event,
  MicState,
  PresetInfo,
} from '../ipc/protocol';
import { t } from '../i18n';
import { meterBus } from './meters';

export interface Toast {
  id: number;
  title: string;
  message?: string;
  tone: 'info' | 'danger';
}

/** Changes closer together than this form one undo step (a slider drag, a wheel spin). */
const UNDO_GESTURE_MS = 600;
const UNDO_DEPTH = 100;
const METER_HZ = 60;

export class DaemonStore {
  readonly client: DaemonClient;
  status = $state<DaemonStatus>({ state: 'connecting' });
  mic = $state.raw<MicState | null>(null);
  presets = $state.raw<PresetInfo[]>([]);
  devices = $state.raw<Device[]>([]);
  ab = $state.raw<AbState | null>(null);
  calibration = $state.raw<CalibrationState>({ state: 'idle' });
  autostart = $state.raw<AutostartState | null>(null);
  toasts = $state<Toast[]>([]);
  canUndo = $state(false);
  canRedo = $state(false);

  private nextToast = 1;
  /** Settings waiting to be sent while a `mic.set` is in flight; the latest wins. */
  private queued: ChainSettings | null = null;
  private inFlight = false;
  private undoStack: ChainSettings[] = [];
  private redoStack: ChainSettings[] = [];
  private lastEditAt = 0;
  private metersOn = false;

  constructor(
    transport: Transport,
    readonly mock: boolean,
  ) {
    this.client = new DaemonClient(transport);
    transport.onStatus((status) => this.onStatus(status));
    transport.onEvent((event) => this.onEvent(event));
    document.addEventListener('visibilitychange', () => this.syncMeters());
  }

  get connected(): boolean {
    return this.status.state === 'connected';
  }

  retry(): void {
    this.client.transport.retry();
  }

  // --- Connection -------------------------------------------------------------------------

  private onStatus(status: DaemonStatus) {
    const was = this.status.state;
    this.status = status;
    if (status.state === 'connected' && was !== 'connected') {
      this.metersOn = false;
      void this.load();
    }
    if (status.state !== 'connected') {
      this.metersOn = false;
      meterBus.reset();
    }
  }

  private async load() {
    try {
      const [mic, presets, devices, ab, calibration, autostart] = await Promise.all([
        this.client.call('mic.get'),
        this.client.call('presets.list'),
        this.client.call('devices.list'),
        this.client.call('ab.get'),
        this.client.call('calibration.get'),
        this.client.call('autostart.get'),
      ]);
      this.mic = mic;
      this.presets = presets;
      this.devices = devices;
      this.ab = ab;
      this.calibration = calibration;
      this.autostart = autostart;
      this.syncMeters();
    } catch (error) {
      this.fail(error);
    }
  }

  /** Meters flow only while the window is visible (MTR-004, NFR-R7). */
  private syncMeters() {
    const want = this.connected && document.visibilityState === 'visible';
    if (want === this.metersOn) return;
    this.metersOn = want;
    const request = want
      ? this.client.call('meters.subscribe', { hz: METER_HZ })
      : this.client.call('meters.unsubscribe');
    request.catch((error) => {
      this.metersOn = !want;
      this.fail(error);
    });
    if (!want) meterBus.reset();
  }

  private onEvent(event: Event) {
    switch (event.event) {
      case 'meters':
        meterBus.latest = event.data;
        meterBus.receivedAt = performance.now();
        break;
      case 'mic.changed':
        this.applyMic(event.data);
        break;
      case 'calibration.changed':
        this.calibration = event.data;
        break;
      case 'ab.changed':
        this.ab = event.data;
        break;
      case 'presets.changed':
        this.presets = event.data;
        break;
    }
  }

  /** While our own edits are pending, their settings are newer than any echo. */
  private applyMic(state: MicState) {
    const previous = this.mic;
    if (this.inFlight || this.queued) {
      if (previous) this.mic = { ...state, settings: previous.settings };
    } else {
      this.mic = state;
    }
    if (previous && previous.input_connected !== state.input_connected && state.device) {
      this.toast(state.input_connected ? t('mic.toastReconnected') : t('mic.toastDisconnected'), {
        tone: state.input_connected ? 'info' : 'danger',
      });
    }
  }

  // --- Chain edits --------------------------------------------------------------------------

  /** Optimistically change the chain and send it; rapid changes coalesce into one request. */
  edit(change: (settings: ChainSettings) => void): void {
    const mic = this.mic;
    if (!mic) return;
    const draft = structuredClone(mic.settings);
    change(draft);
    this.recordUndo(mic.settings);
    this.mic = { ...mic, settings: draft };
    this.send(draft);
  }

  private recordUndo(before: ChainSettings) {
    const now = performance.now();
    if (now - this.lastEditAt > UNDO_GESTURE_MS) {
      this.undoStack.push(structuredClone(before));
      if (this.undoStack.length > UNDO_DEPTH) this.undoStack.shift();
      this.redoStack = [];
    }
    this.lastEditAt = now;
    this.syncHistory();
  }

  private syncHistory() {
    this.canUndo = this.undoStack.length > 0;
    this.canRedo = this.redoStack.length > 0;
  }

  /** DSP-008: undo and redo restore the chain but keep the current mute and bypass. */
  undo(): void {
    this.travel(this.undoStack, this.redoStack);
  }

  redo(): void {
    this.travel(this.redoStack, this.undoStack);
  }

  private travel(from: ChainSettings[], to: ChainSettings[]) {
    const mic = this.mic;
    const target = from.pop();
    if (!mic || !target) return;
    to.push(structuredClone(mic.settings));
    const settings = { ...target, muted: mic.settings.muted, bypass: mic.settings.bypass };
    this.lastEditAt = 0;
    this.mic = { ...mic, settings };
    this.send(settings);
    this.syncHistory();
  }

  private send(settings: ChainSettings) {
    if (this.inFlight) {
      this.queued = settings;
      return;
    }
    this.inFlight = true;
    this.client
      .call('mic.set', { settings })
      .then((state) => {
        if (!this.queued) this.mic = state;
      })
      .catch((error) => {
        this.fail(error);
        // Resynchronise with the daemon after a rejected change.
        void this.client.call('mic.get').then((state) => (this.mic = state));
      })
      .finally(() => {
        this.inFlight = false;
        const next = this.queued;
        this.queued = null;
        if (next) this.send(next);
      });
  }

  // --- Discrete actions ---------------------------------------------------------------------

  async run<T>(request: Promise<T>): Promise<T | undefined> {
    try {
      return await request;
    } catch (error) {
      this.fail(error);
      return undefined;
    }
  }

  async setMic(request: Promise<MicState>): Promise<void> {
    const state = await this.run(request);
    if (state) this.mic = state;
  }

  async selectPreset(id: string): Promise<void> {
    if (this.mic) this.recordUndo(this.mic.settings);
    this.lastEditAt = 0;
    await this.setMic(this.client.call('mic.set_preset', { preset: id }));
  }

  async refreshDevices(): Promise<void> {
    const devices = await this.run(this.client.call('devices.list'));
    if (devices) this.devices = devices;
  }

  // --- Toasts -------------------------------------------------------------------------------

  toast(title: string, options: { message?: string; tone?: Toast['tone'] } = {}): void {
    const id = this.nextToast++;
    this.toasts.push({ id, title, message: options.message, tone: options.tone ?? 'info' });
    setTimeout(() => this.dismiss(id), 5000);
  }

  dismiss(id: number): void {
    const index = this.toasts.findIndex((toast) => toast.id === id);
    if (index >= 0) this.toasts.splice(index, 1);
  }

  fail(error: unknown): void {
    if (error instanceof DaemonError) {
      const message = error.code === 'already_exists' ? t('errors.alreadyExists') : error.message;
      this.toast(t('errors.generic'), { message, tone: 'danger' });
    } else {
      this.toast(t('errors.generic'), { message: String(error), tone: 'danger' });
    }
  }
}
