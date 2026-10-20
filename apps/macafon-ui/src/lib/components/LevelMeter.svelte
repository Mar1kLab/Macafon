<script lang="ts">
  // Segmented level meter (design system §2.2, §6): 2px segments with 1px gaps, RMS solid,
  // peak dimmer, a held peak tick and a clip latch (MTR-001).
  import { onMount } from 'svelte';
  import { cssVar, drawEveryFrame } from '../state/frame';
  import { Ballistics, meterBus, type MeterKey } from '../state/meters';
  import { formatLevel } from '../dsp/format';
  import { t } from '../i18n';

  interface Props {
    label: string;
    peakKey: MeterKey;
    rmsKey?: MeterKey;
    /** Separate key for the clip latch (the raw input clips before Gain, MIC-G1). */
    clipKey?: MeterKey;
    color?: string;
    hint?: string;
    onclip?: (clipped: boolean) => void;
  }

  let { label, peakKey, rmsKey, clipKey, color = 'blue', hint, onclip }: Props = $props();

  const MIN_DB = -60;
  const TEXT_INTERVAL_MS = 100;
  let canvas: HTMLCanvasElement | undefined = $state();
  let readout: HTMLSpanElement | undefined = $state();
  let clipped = $state(false);

  onMount(() => {
    const peak = new Ballistics();
    const rms = new Ballistics();
    const clip = new Ballistics(-0.1);
    let colors: Record<string, string> = {};
    let lastText = '';
    let textAt = -Infinity;
    const readColors = () => {
      const element = canvas!;
      colors = {
        ink: cssVar(element, `--app-${color}-ink`),
        amber: cssVar(element, '--app-amber-ink'),
        danger: cssVar(element, '--danger'),
        empty: cssVar(element, '--line'),
      };
    };
    readColors();
    const colorAt = (db: number) => (db > -1 ? colors.danger! : db > -6 ? colors.amber! : colors.ink!);
    return drawEveryFrame(canvas!, (surface, now) => {
      const frame = meterBus.latest;
      peak.update(frame[peakKey], now);
      rms.update(rmsKey ? frame[rmsKey] : frame[peakKey], now);
      clip.update(frame[clipKey ?? peakKey], now);
      if (clip.clipped !== clipped) {
        clipped = clip.clipped;
        onclip?.(clipped);
      }
      const text = formatLevel(peak.level);
      // Numbers are read, not watched: 10 updates a second, each one a text layout.
      if (readout && text !== lastText && now - textAt >= TEXT_INTERVAL_MS) {
        readout.textContent = text;
        lastText = text;
        textAt = now;
      }
      const { width, height } = surface;
      const x = (db: number) => Math.min(Math.max((db - MIN_DB) / -MIN_DB, 0), 1) * width;
      // Segments start every 3 px: levels within one segment draw the same picture.
      const peakX = Math.ceil(x(peak.level) / 3);
      const rmsX = Math.ceil(x(rms.level) / 3);
      const tick = peak.peak > MIN_DB ? Math.round(Math.min(x(peak.peak), width - 2)) : -1;
      const context = surface.begin(`${peakX}|${rmsX}|${tick}|${colorAt(peak.peak)}`);
      if (!context) return;
      for (let left = 0; left < width; left += 3) {
        const db = MIN_DB + (left / width) * -MIN_DB;
        const segment = left / 3;
        if (segment < rmsX) {
          context.globalAlpha = db < -18 ? 0.6 : 1;
          context.fillStyle = colorAt(db);
        } else if (segment < peakX) {
          context.globalAlpha = 0.3;
          context.fillStyle = colorAt(db);
        } else {
          context.globalAlpha = 1;
          context.fillStyle = colors.empty!;
        }
        context.fillRect(left, 0, 2, height);
      }
      context.globalAlpha = 1;
      if (tick >= 0) {
        context.fillStyle = colorAt(peak.peak);
        context.fillRect(tick, 0, 2, height);
      }
    });
  });
</script>

<div class="meter" title={hint}>
  <span class="hud-label">{label}</span>
  <canvas bind:this={canvas} aria-hidden="true"></canvas>
  <!-- Hidden unless clipping, but keeps its width so the readout does not jump. -->
  <span class="clip" class:on={clipped} aria-hidden={!clipped}>{t('meter.clip')}</span>
  <span class="value level" bind:this={readout}></span>
  <span class="unit hud-label">dB</span>
</div>

<style>
  .meter {
    display: grid;
    grid-template-columns: auto 1fr auto 48px auto;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  canvas {
    width: 100%;
    height: 10px;
    display: block;
  }

  .level {
    text-align: right;
  }

  .unit {
    color: var(--faint);
  }

  .clip {
    font-family: var(--font-mono);
    font-size: 9px;
    letter-spacing: 0.1em;
    padding: 0 4px;
    border: 1px solid transparent;
    visibility: hidden;
  }

  .clip.on {
    visibility: visible;
    color: var(--invert-fg);
    background: var(--danger);
    border-color: var(--danger);
  }
</style>
