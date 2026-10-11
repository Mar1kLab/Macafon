// Value formatting for HUD readouts and `aria-valuetext` (PRD §9.8).

import type { Unit } from '../ipc/dsp';

const MINUS = '−';

function signed(value: number, digits: number): string {
  const text = Math.abs(value).toFixed(digits);
  if (Number(text) === 0) return text;
  return value < 0 ? `${MINUS}${text}` : `+${text}`;
}

export function formatHz(hz: number): string {
  if (hz >= 1000) {
    const k = hz / 1000;
    return `${k >= 10 ? k.toFixed(1) : k.toFixed(2)} kHz`.replace(/\.?0+ kHz$/, ' kHz');
  }
  return `${Math.round(hz)} Hz`;
}

export function formatValue(value: number, unit: Unit): string {
  switch (unit) {
    case 'percent':
      return `${Math.round(value)}%`;
    case 'db':
      return `${signed(value, 1)} dB`;
    case 'dbfs':
      return `${signed(value, 1).replace(/^\+/, '')} dBFS`;
    case 'hz':
      return formatHz(value);
    case 'ms':
      return value < 10 ? `${value.toFixed(1)} ms` : `${Math.round(value)} ms`;
    case 'ratio':
      return `${value.toFixed(1)}:1`;
    case 'factor':
      return value.toFixed(2);
  }
}

/** Meter readout: silence shows as −∞. */
export function formatLevel(dbfs: number): string {
  if (!Number.isFinite(dbfs) || dbfs <= -96) return `${MINUS}∞`;
  return signed(dbfs, 1).replace(/^\+/, '');
}

export function formatDb(db: number, digits = 1): string {
  return `${signed(db, digits)} dB`;
}

export function formatMs(ms: number): string {
  return `${(ms / 1000).toFixed(1)} s`;
}

export function samplesToMs(samples: number, sampleRateHz: number): number {
  return (samples / sampleRateHz) * 1000;
}

/** CH-004: the cubic volume curve of `macafon_dsp::channel::volume_db`, shown next to the %. */
export function volumeDb(volumePct: number): number {
  return 60 * Math.log10(Math.min(Math.max(volumePct, 0), 100) / 100);
}

export function formatVolumeDb(volumePct: number): string {
  const db = volumeDb(volumePct);
  return Number.isFinite(db) ? formatDb(db) : `${MINUS}∞ dB`;
}
