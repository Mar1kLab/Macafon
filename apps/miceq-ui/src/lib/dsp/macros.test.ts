import { describe, expect, it } from 'vitest';
import { BUILTIN_PRESETS, DEFAULT_NOISE_FLOOR_DBFS } from '../ipc/dsp';
import {
  compressorAmount,
  compressorFromAmount,
  deEsserIntensity,
  eqCharacter,
  eqFromCharacter,
  expanderFromSensitivity,
  expanderSensitivity,
} from './macros';

const preset = (id: string) => BUILTIN_PRESETS.find((p) => p.id === id)!.settings;

// The presets are built by miceq-dsp from the same macros, so inverting them must land on the
// slider values of PRD §6.3.10 — a mismatch means the TS mirror drifted from the Rust code.
describe('macros mirror miceq-dsp', () => {
  it('places the sliders of the built-in presets', () => {
    const natural = preset('natural');
    expect(compressorAmount(natural.compressor)).toBeCloseTo(30, 3);
    expect(deEsserIntensity(natural.de_esser)).toBeCloseTo(30, 3);
    expect(expanderSensitivity(natural.expander)).toBeCloseTo(40, 3);
    expect(eqCharacter(natural.eq)).toEqual({ character: 'flat', amountPct: 100 });

    const discord = preset('discord-clear');
    expect(compressorAmount(discord.compressor)).toBeCloseTo(50, 3);
    expect(expanderSensitivity(discord.expander)).toBeCloseTo(60, 3);
    expect(eqCharacter(discord.eq)?.character).toBe('clarity');
    expect(eqCharacter(discord.eq)?.amountPct).toBeCloseTo(50, 3);

    const broadcast = preset('broadcast');
    expect(compressorAmount(broadcast.compressor)).toBeCloseTo(65, 3);
    expect(eqCharacter(broadcast.eq)?.amountPct).toBeCloseTo(70, 3);
  });

  it('reproduces the Rust parameters for a slider position', () => {
    const natural = preset('natural');
    const compressor = compressorFromAmount(30);
    expect(compressor.ratio).toBeCloseTo(natural.compressor.ratio, 4);
    expect(compressor.threshold_db).toBeCloseTo(natural.compressor.threshold_db, 4);
    const expander = expanderFromSensitivity(40, DEFAULT_NOISE_FLOOR_DBFS);
    expect(expander.threshold_db).toBeCloseTo(natural.expander.threshold_db, 4);
    expect(expander.range_db).toBeCloseTo(natural.expander.range_db, 4);
    const eq = eqFromCharacter('clarity', 50);
    preset('discord-clear').eq.bands.forEach((band, i) => expect(eq.bands[i]!.gain_db).toBeCloseTo(band.gain_db, 4));
  });

  it('reports custom settings (DSP-002)', () => {
    const settings = preset('natural');
    expect(compressorAmount({ ...settings.compressor, attack_ms: 20 })).toBeNull();
    expect(compressorAmount({ ...settings.compressor, makeup_db: 3 })).toBeNull();
    expect(deEsserIntensity({ ...settings.de_esser, mode: 'wideband' })).toBeNull();
    expect(deEsserIntensity({ ...settings.de_esser, listen: true })).toBeCloseTo(30, 3);
    expect(expanderSensitivity({ ...settings.expander, ratio: 'gate' })).toBeNull();
    // Calibration moves only the threshold; the expander stays in simple mode.
    expect(expanderSensitivity({ ...settings.expander, threshold_db: -47.5 })).toBeCloseTo(40, 3);
    const eq = eqFromCharacter('presence', 40);
    eq.bands[3] = { ...eq.bands[3]!, gain_db: 5 };
    expect(eqCharacter(eq)).toBeNull();
  });
});
