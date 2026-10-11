<script lang="ts">
  // Gain-reduction mini meter (DSP-004, MTR-002): grows from the right as the module cuts.
  // Also shows NS activity 0…1 (MTR-003) when `activity` is set.
  import { onMount } from 'svelte';
  import { cssVar, fitCanvas, onFrame } from '../state/frame';
  import { meterBus, type MeterKey } from '../state/meters';
  import { t } from '../i18n';

  interface Props {
    /** Microphone meter to show; or `read` for any other source (channel reduction). */
    meterKey?: MeterKey;
    read?: () => number;
    color: string;
    activity?: boolean;
    active?: boolean;
  }

  let { meterKey, read, color, activity = false, active = true }: Props = $props();
  const value = () => (read ? read() : meterKey ? meterBus.latest[meterKey] : 0);

  const RANGE_DB = 24;
  let canvas: HTMLCanvasElement | undefined = $state();
  let readout: HTMLSpanElement | undefined = $state();

  onMount(() => {
    let shown = 0;
    let last = 0;
    let lastText = '';
    const ink = cssVar(canvas!, `--app-${color}-ink`);
    const empty = cssVar(canvas!, '--line');
    return onFrame((now) => {
      if (!canvas) return;
      const dt = last ? (now - last) / 1000 : 0;
      last = now;
      const raw = active ? value() : 0;
      // Fraction of the meter: activity is 0…1, reduction is negative dB.
      const target = activity ? Math.min(Math.max(raw, 0), 1) : Math.min(-raw / RANGE_DB, 1);
      // Reduction rises instantly and falls back at 20 dB/s, like the level meters.
      const fall = (20 / RANGE_DB) * dt;
      shown = target >= shown ? target : Math.max(target, shown - fall);
      const context = fitCanvas(canvas);
      if (!context) return;
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      context.clearRect(0, 0, width, height);
      const filled = shown * width;
      for (let left = 0; left < width; left += 3) {
        const fromRight = width - left - 2;
        context.fillStyle = fromRight < filled ? ink : empty;
        context.fillRect(left, 0, 2, height);
      }
      const text = activity
        ? `${Math.round(shown * 100)}%`
        : shown * RANGE_DB < 0.05
          ? '0.0 dB'
          : `−${(shown * RANGE_DB).toFixed(1)} dB`;
      if (readout && text !== lastText) {
        readout.textContent = text;
        lastText = text;
      }
    });
  });
</script>

<div class="gr" class:inactive={!active} title={activity ? t('module.activityHint') : t('module.reductionHint')}>
  <span class="hud-label">{activity ? t('module.activity') : t('module.reduction')}</span>
  <canvas bind:this={canvas} aria-hidden="true"></canvas>
  <span class="value" bind:this={readout}></span>
</div>

<style>
  .gr {
    display: grid;
    grid-template-columns: 44px 1fr 56px;
    align-items: center;
    gap: 8px;
  }

  canvas {
    width: 100%;
    height: 6px;
    display: block;
  }

  .value {
    text-align: right;
    font-size: 11px;
    color: var(--muted);
  }

  .inactive {
    opacity: 0.4;
  }
</style>
