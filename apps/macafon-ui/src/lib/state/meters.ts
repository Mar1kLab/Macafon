// Latest meter frame, outside Svelte reactivity: canvases read it in their animation frame.
// Ballistics follow the design system §5: peak hold 1 s, fall 20 dB/s, clip latch 2 s.

import type { Meters } from '../ipc/protocol';

export const SILENCE_DB = -120;

const silent: Meters = {
  raw_peak_dbfs: SILENCE_DB,
  input_peak_dbfs: SILENCE_DB,
  input_rms_dbfs: SILENCE_DB,
  output_peak_dbfs: SILENCE_DB,
  output_rms_dbfs: SILENCE_DB,
  expander_reduction_db: 0,
  compressor_reduction_db: 0,
  de_esser_reduction_db: 0,
  limiter_reduction_db: 0,
  noise_activity: 0,
};

export const meterBus = {
  latest: { ...silent } as Meters,
  receivedAt: 0,
  reset() {
    this.latest = { ...silent };
  },
};

export type MeterKey = {
  [K in keyof Meters]-?: Meters[K] extends number ? K : never;
}[keyof Meters];

const PEAK_HOLD_MS = 1000;
const FALL_DB_PER_S = 20;
const CLIP_LATCH_MS = 2000;
const CLIP_DBFS = -1;

/** Smooths one level for display: instant rise, linear fall, held peak and clip latch. */
export class Ballistics {
  level = SILENCE_DB;
  peak = SILENCE_DB;
  clipped = false;
  private peakAt = 0;
  private clipAt = -Infinity;
  private last = 0;

  constructor(private readonly clipDbfs = CLIP_DBFS) {}

  update(valueDb: number, now: number): void {
    const dt = this.last ? (now - this.last) / 1000 : 0;
    this.last = now;
    const value = Number.isFinite(valueDb) ? valueDb : SILENCE_DB;
    this.level = value >= this.level ? value : Math.max(value, this.level - FALL_DB_PER_S * dt);
    if (value >= this.peak) {
      this.peak = value;
      this.peakAt = now;
    } else if (now - this.peakAt > PEAK_HOLD_MS) {
      this.peak = Math.max(this.level, this.peak - FALL_DB_PER_S * dt);
    }
    if (value > this.clipDbfs) this.clipAt = now;
    this.clipped = now - this.clipAt < CLIP_LATCH_MS;
  }
}
