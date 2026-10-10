// Main-slider macros (DSP-002). The curves come from macafon-dsp (dsp.ts); these functions mirror
// `ExpanderParams::from_sensitivity`, `CompressorParams::from_amount`,
// `DeEsserParams::from_intensity` and `EqParams::from_character`, and invert them to place
// the slider. Settings that no slider position produces are CUSTOM.

import {
  DEFAULT_NOISE_FLOOR_DBFS,
  EQ_CHARACTERS,
  EQ_DEFAULT_LAYOUT,
  MACROS,
  SPECS,
  type Curve,
} from '../ipc/dsp';
import type {
  ChainSettings,
  CompressorParams,
  DeEsserParams,
  EqParams,
  ExpanderParams,
} from '../ipc/protocol';

export type EqCharacterId = (typeof EQ_CHARACTERS)[number]['id'];

/** Piecewise-linear interpolation, as `macafon_dsp::util::interpolate`. */
export function interpolate(x: number, curve: Curve): number {
  let previous = curve[0]!;
  if (x <= previous[0]) return previous[1];
  for (const point of curve.slice(1)) {
    if (x <= point[0]) {
      const t = (x - previous[0]) / (point[0] - previous[0]);
      return previous[1] + (point[1] - previous[1]) * t;
    }
    previous = point;
  }
  return previous[1];
}

/** Slider position (0…1) of a value on a monotonic curve; `null` outside the curve. */
export function invert(y: number, curve: Curve): number | null {
  for (let i = 1; i < curve.length; i++) {
    const [x0, y0] = curve[i - 1]!;
    const [x1, y1] = curve[i]!;
    const low = Math.min(y0, y1);
    const high = Math.max(y0, y1);
    if (y >= low - 1e-4 && y <= high + 1e-4) {
      if (y0 === y1) return x0;
      return Math.min(Math.max(x0 + ((y - y0) / (y1 - y0)) * (x1 - x0), 0), 1);
    }
  }
  return null;
}

/** The daemon computes in f32: compare with a relative tolerance. */
export function close(a: number, b: number): boolean {
  return Math.abs(a - b) <= 1e-3 * Math.max(1, Math.abs(a), Math.abs(b));
}

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max);

// --- Expander -------------------------------------------------------------------------------

/**
 * Level the Sensitivity slider is relative to: the calibrated noise floor after Gain (the
 * expander follows Gain, HPF and NS), or the default floor the built-in presets assume.
 */
export function expanderReferenceDbfs(
  settings: ChainSettings,
  noiseFloorDbfs: number | null,
): number {
  if (noiseFloorDbfs === null) return DEFAULT_NOISE_FLOOR_DBFS;
  return noiseFloorDbfs + (settings.gain.enabled ? settings.gain.gain_db : 0);
}

export function expanderFromSensitivity(pct: number, referenceDbfs: number): ExpanderParams {
  const s = pct * 0.01;
  return {
    threshold_db: clamp(
      referenceDbfs + interpolate(s, MACROS.expander_threshold_offset_db),
      SPECS.expander_threshold_db.min,
      SPECS.expander_threshold_db.max,
    ),
    ratio: interpolate(s, MACROS.expander_ratio),
    range_db: interpolate(s, MACROS.expander_range_db),
    attack_ms: SPECS.expander_attack_ms.default,
    hold_ms: SPECS.expander_hold_ms.default,
    release_ms: SPECS.expander_release_ms.default,
    hysteresis_db: SPECS.expander_hysteresis_db.default,
  };
}

/**
 * Sensitivity of expander settings, or `null` when they are custom. The threshold is not
 * compared: calibration moves it to the measured floor (CAL-003) without leaving simple mode.
 */
export function expanderSensitivity(params: ExpanderParams): number | null {
  if (params.ratio === 'gate') return null;
  const s = invert(params.ratio, MACROS.expander_ratio);
  if (s === null) return null;
  const expected = expanderFromSensitivity(s * 100, 0);
  const same =
    close(params.range_db, expected.range_db) &&
    close(params.attack_ms, expected.attack_ms) &&
    close(params.hold_ms, expected.hold_ms) &&
    close(params.release_ms, expected.release_ms) &&
    close(params.hysteresis_db, expected.hysteresis_db);
  return same ? s * 100 : null;
}

export function nearestExpanderSensitivity(params: ExpanderParams): number {
  if (params.ratio === 'gate') return 100;
  return (invert(clamp(params.ratio, 1.5, 6), MACROS.expander_ratio) ?? 0.5) * 100;
}

// --- Compressor -----------------------------------------------------------------------------

export function compressorFromAmount(pct: number): CompressorParams {
  const a = pct * 0.01;
  return {
    threshold_db: interpolate(a, MACROS.compressor_threshold_db),
    ratio: interpolate(a, MACROS.compressor_ratio),
    attack_ms: SPECS.compressor_attack_ms.default,
    release_ms: SPECS.compressor_release_ms.default,
    knee_db: SPECS.compressor_knee_db.default,
    makeup_db: null,
    detector: 'rms',
  };
}

export function compressorAmount(params: CompressorParams): number | null {
  const a = invert(params.threshold_db, MACROS.compressor_threshold_db);
  if (a === null) return null;
  const expected = compressorFromAmount(a * 100);
  const same =
    close(params.ratio, expected.ratio) &&
    close(params.attack_ms, expected.attack_ms) &&
    close(params.release_ms, expected.release_ms) &&
    close(params.knee_db, expected.knee_db) &&
    params.makeup_db === null &&
    params.detector === expected.detector;
  return same ? a * 100 : null;
}

export function nearestCompressorAmount(params: CompressorParams): number {
  const curve = MACROS.compressor_threshold_db;
  const threshold = clamp(params.threshold_db, curve[curve.length - 1]![1], curve[0][1]);
  return (invert(threshold, curve) ?? 0.4) * 100;
}

// --- De-esser -------------------------------------------------------------------------------

export function deEsserFromIntensity(pct: number): DeEsserParams {
  return {
    freq_hz: SPECS.de_esser_freq_hz.default,
    threshold_db: interpolate(pct * 0.01, MACROS.de_esser_threshold_db),
    max_reduction_db: SPECS.de_esser_max_reduction_db.default,
    mode: 'split_band',
    listen: false,
  };
}

/** Listen is a monitoring switch, not a setting the macro owns. */
export function deEsserIntensity(params: DeEsserParams): number | null {
  const i = invert(params.threshold_db, MACROS.de_esser_threshold_db);
  if (i === null) return null;
  const expected = deEsserFromIntensity(i * 100);
  const same =
    close(params.freq_hz, expected.freq_hz) &&
    close(params.max_reduction_db, expected.max_reduction_db) &&
    params.mode === expected.mode;
  return same ? i * 100 : null;
}

export function nearestDeEsserIntensity(params: DeEsserParams): number {
  const curve = MACROS.de_esser_threshold_db;
  const threshold = clamp(params.threshold_db, curve[curve.length - 1]![1], curve[0][1]);
  return (invert(threshold, curve) ?? 0.4) * 100;
}

// --- EQ (EQ-002) ----------------------------------------------------------------------------

export function eqFromCharacter(id: EqCharacterId, amountPct: number): EqParams {
  const character = EQ_CHARACTERS.find((c) => c.id === id) ?? EQ_CHARACTERS[0];
  return {
    bands: EQ_DEFAULT_LAYOUT.map((band, i) => ({
      ...band,
      gain_db: (character.gains_db[i] ?? 0) * amountPct * 0.01,
    })),
    output_gain_db: SPECS.eq_output_gain_db.default,
  };
}

export interface EqMacro {
  character: EqCharacterId;
  amountPct: number;
}

/** Character and amount of an EQ, or `null` when its bands were edited (custom). */
export function eqCharacter(params: EqParams): EqMacro | null {
  if (params.bands.length !== EQ_DEFAULT_LAYOUT.length) return null;
  if (!close(params.output_gain_db, SPECS.eq_output_gain_db.default)) return null;
  const layoutMatches = params.bands.every((band, i) => {
    const layout = EQ_DEFAULT_LAYOUT[i]!;
    return band.filter === layout.filter && close(band.freq_hz, layout.freq_hz) && close(band.q, layout.q);
  });
  if (!layoutMatches) return null;
  const gains = params.bands.map((band) => band.gain_db);
  if (gains.every((gain) => Math.abs(gain) < 1e-4)) {
    return { character: 'flat', amountPct: SPECS.eq_amount_pct.default };
  }
  for (const character of EQ_CHARACTERS) {
    const reference = character.gains_db;
    // The band with the largest gain gives the amount most precisely.
    let index = 0;
    reference.forEach((gain, i) => {
      if (Math.abs(gain) > Math.abs(reference[index]!)) index = i;
    });
    if (reference[index] === 0) continue;
    const amount = (gains[index]! / reference[index]!) * 100;
    if (amount < -1e-3 || amount > 100 + 1e-3) continue;
    if (gains.every((gain, i) => Math.abs(gain - (reference[i] ?? 0) * amount * 0.01) < 1e-3)) {
      return { character: character.id, amountPct: clamp(amount, 0, 100) };
    }
  }
  return null;
}
