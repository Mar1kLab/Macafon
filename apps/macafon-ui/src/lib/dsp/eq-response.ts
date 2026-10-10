// Magnitude response of the EQ for the graph (EQ-001). Mirrors the coefficient formulas of
// `macafon_dsp::svf::coefficients`: a trapezoidal SVF equals its analog prototype evaluated at
// s = j·tan(πf/fs)/g, so H = m0 + (m1·s + m2) / (s² + k·s + 1).

import { SAMPLE_RATE_HZ } from '../ipc/dsp';
import type { EqBand, EqFilter } from '../ipc/protocol';

type Coefficients = [g: number, k: number, m0: number, m1: number, m2: number];

function coefficients(filter: EqFilter, freqHz: number, q: number, gainDb: number): Coefficients {
  const freq = Math.min(Math.max(freqHz, 1), SAMPLE_RATE_HZ * 0.49);
  const g = Math.tan((Math.PI * freq) / SAMPLE_RATE_HZ);
  const k = 1 / q;
  const a = 10 ** (gainDb / 40);
  switch (filter) {
    case 'bell': {
      const kb = 1 / (q * a);
      return [g, kb, 1, kb * (a * a - 1), 0];
    }
    case 'low_shelf':
      return [g / Math.sqrt(a), k, 1, k * (a - 1), a * a - 1];
    case 'high_shelf':
      return [g * Math.sqrt(a), k, a * a, k * (1 - a) * a, 1 - a * a];
    case 'low_cut':
      return [g, k, 1, -k, -1];
    case 'high_cut':
      return [g, k, 0, 0, 1];
    case 'notch':
      return [g, k, 1, -k, 0];
  }
}

/** Gain in dB of one band at `freqHz`. */
export function bandGainDb(band: EqBand, freqHz: number): number {
  const [g, k, m0, m1, m2] = coefficients(band.filter, band.freq_hz, band.q, band.gain_db);
  const w = Math.tan((Math.PI * Math.min(freqHz, SAMPLE_RATE_HZ * 0.4999)) / SAMPLE_RATE_HZ) / g;
  // s = jw: s² + k·s + 1 = (1 − w²) + j·k·w; m1·s + m2 = m2 + j·m1·w.
  const dRe = 1 - w * w;
  const dIm = k * w;
  const nRe = m2;
  const nIm = m1 * w;
  const den = dRe * dRe + dIm * dIm;
  const re = m0 + (nRe * dRe + nIm * dIm) / den;
  const im = (nIm * dRe - nRe * dIm) / den;
  return 10 * Math.log10(Math.max(re * re + im * im, 1e-24));
}

/** Total response of all bands plus output gain. */
export function eqGainDb(bands: readonly EqBand[], outputGainDb: number, freqHz: number): number {
  return bands.reduce((sum, band) => sum + bandGainDb(band, freqHz), outputGainDb);
}

/** Logarithmic frequency axis of the graph. */
export const GRAPH_MIN_HZ = 20;
export const GRAPH_MAX_HZ = 20_000;

export function freqToX(freqHz: number): number {
  return Math.log(freqHz / GRAPH_MIN_HZ) / Math.log(GRAPH_MAX_HZ / GRAPH_MIN_HZ);
}

export function xToFreq(x: number): number {
  return GRAPH_MIN_HZ * (GRAPH_MAX_HZ / GRAPH_MIN_HZ) ** x;
}
