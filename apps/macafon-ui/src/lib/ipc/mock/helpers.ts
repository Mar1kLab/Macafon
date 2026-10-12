// Pure protocol-state helpers for the browser mock.
import { LITE_LATENCY_SAMPLES, SAMPLE_RATE_HZ, STUDIO_LATENCY_SAMPLES } from '../dsp';
import type { ChainSettings, ChannelSettings } from '../protocol';

export function channelLatency(settings: ChannelSettings): number {
  if (settings.bypass) return 0;
  return settings.noise_suppression.enabled ? LITE_LATENCY_SAMPLES : 0;
}

export function clone<T>(value: T): T {
  return structuredClone(value);
}

/** Chain equality without mute and bypass, which presets do not store. */
export function sameChain(a: ChainSettings, b: ChainSettings): boolean {
  const strip = (s: ChainSettings) => JSON.stringify({ ...s, muted: false, bypass: false });
  return strip(a) === strip(b);
}

export function latencySamples(settings: ChainSettings): number {
  if (settings.bypass) return 0;
  const engine = settings.noise_suppression.engine === 'studio' ? STUDIO_LATENCY_SAMPLES : LITE_LATENCY_SAMPLES;
  const ns = settings.noise_suppression.enabled ? engine : 0;
  const limiter = settings.limiter.enabled ? (settings.limiter.lookahead_ms * SAMPLE_RATE_HZ) / 1000 : 0;
  return ns + limiter;
}

export function slugify(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'preset';
}

