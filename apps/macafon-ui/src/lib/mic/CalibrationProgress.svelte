<script lang="ts">
  // Calibration in progress (§6.5, §9.2 step 3): silence → sentence with a large circular
  // progress and a live level trace.
  import { onMount } from 'svelte';
  import { cssVar, drawEveryFrame } from '../state/frame';
  import { meterBus } from '../state/meters';
  import { t } from '../i18n';
  import type { CalibrationState } from '../ipc/protocol';

  let { state: cal }: { state: Extract<CalibrationState, { state: 'measuring' }> } = $props();

  const SILENCE_SHARE = 0.4;
  const RADIUS = 54;
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS;
  let canvas: HTMLCanvasElement | undefined = $state();

  const overall = $derived(
    cal.step === 'silence' ? cal.progress * SILENCE_SHARE : SILENCE_SHARE + cal.progress * (1 - SILENCE_SHARE),
  );
  const remaining = $derived(Math.ceil(((1 - cal.progress) * cal.step_duration_ms) / 1000));

  onMount(() => {
    const history = new Float32Array(160);
    let index = 0;
    const ink = cssVar(canvas!, '--accent-ink');
    return drawEveryFrame(canvas!, (surface) => {
      history[index] = meterBus.latest.raw_peak_dbfs;
      index = (index + 1) % history.length;
      const context = surface.begin();
      if (!context) return;
      const { width, height } = surface;
      context.fillStyle = ink;
      const step = width / history.length;
      for (let i = 0; i < history.length; i++) {
        const db = history[(index + i) % history.length]!;
        const level = Math.min(Math.max((db + 70) / 70, 0), 1);
        const bar = Math.max(1, level * height);
        context.globalAlpha = 0.35 + 0.65 * (i / history.length);
        context.fillRect(i * step, (height - bar) / 2, Math.max(1, step - 1), bar);
      }
      context.globalAlpha = 1;
    });
  });
</script>

<div class="measuring">
  <svg viewBox="0 0 128 128" class="ring" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(overall * 100)}>
    <circle cx="64" cy="64" r={RADIUS} class="track" />
    <circle
      cx="64"
      cy="64"
      r={RADIUS}
      class="progress"
      stroke-dasharray={CIRCUMFERENCE}
      stroke-dashoffset={CIRCUMFERENCE * (1 - overall)}
    />
    <text x="64" y="70" text-anchor="middle" class="count">{remaining}</text>
  </svg>
  <div class="step">
    <span class="hud-label">{cal.step === 'silence' ? '01 / 02' : '02 / 02'}</span>
    <h3>{cal.step === 'silence' ? t('calibration.silence') : t('calibration.speech')}</h3>
    {#if cal.step === 'silence'}
      <p class="note">{t('calibration.silenceHint')}</p>
    {:else}
      <p class="sentence">“{t('calibration.sentence')}”</p>
    {/if}
  </div>
</div>
<canvas bind:this={canvas} class="trace" aria-hidden="true"></canvas>

<style>
  .note {
    margin-top: 12px;
    font-size: 13px;
    color: var(--muted);
  }

  .measuring {
    display: grid;
    grid-template-columns: 128px 1fr;
    gap: 24px;
    align-items: center;
  }

  .ring {
    width: 128px;
    height: 128px;
    transform: rotate(-90deg);
  }

  .ring .track {
    fill: none;
    stroke: var(--line);
    stroke-width: 4;
  }

  .ring .progress {
    fill: none;
    stroke: var(--accent);
    stroke-width: 4;
    transition: stroke-dashoffset 120ms linear;
  }

  .count {
    transform: rotate(90deg);
    transform-origin: 64px 64px;
    fill: var(--ink-strong);
    font-family: var(--font-display);
    font-size: 28px;
    font-weight: 600;
  }

  h3 {
    margin-top: 4px;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 600;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
  }

  .sentence {
    margin-top: 8px;
    font-family: var(--font-serif);
    font-style: italic;
    font-size: 20px;
    line-height: 1.3;
    color: var(--ink);
  }

  .trace {
    display: block;
    width: 100%;
    height: 40px;
    margin-top: 20px;
    background: var(--bg-sunken);
    border: 1px solid var(--line);
  }
</style>
