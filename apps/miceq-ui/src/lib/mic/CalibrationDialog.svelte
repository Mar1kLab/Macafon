<script lang="ts">
  // Calibration (§6.5): silence → sentence with a large circular progress and a live level
  // trace, then the result (CAL-004) or the failure with a retry (CAL-005, CAL-006).
  import { onMount } from 'svelte';
  import Dialog from '../components/Dialog.svelte';
  import { daemon } from '../state';
  import { cssVar, fitCanvas, onFrame } from '../state/frame';
  import { meterBus } from '../state/meters';
  import { formatDb, formatHz, formatValue } from '../dsp/format';
  import { MODULES } from './modules';
  import { t } from '../i18n';

  let { open, onclose, onlisten }: { open: boolean; onclose: () => void; onlisten: () => void } = $props();

  const SILENCE_SHARE = 0.4;
  let started = $state(false);
  let canvas: HTMLCanvasElement | undefined = $state();

  const cal = $derived(daemon.calibration);
  const phase = $derived(started ? cal.state : 'idle');
  const overall = $derived(
    cal.state === 'measuring'
      ? cal.step === 'silence'
        ? cal.progress * SILENCE_SHARE
        : SILENCE_SHARE + cal.progress * (1 - SILENCE_SHARE)
      : cal.state === 'done'
        ? 1
        : 0,
  );
  const remaining = $derived(
    cal.state === 'measuring' ? Math.ceil(((1 - cal.progress) * cal.step_duration_ms) / 1000) : 0,
  );
  const RADIUS = 54;
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

  $effect(() => {
    if (!open) started = false;
  });

  async function start() {
    started = true;
    const result = await daemon.run(daemon.client.call('calibration.start'));
    if (result) daemon.calibration = result;
    else started = false;
  }

  function close() {
    if (cal.state === 'measuring') void daemon.run(daemon.client.call('calibration.cancel'));
    onclose();
  }

  onMount(() => {
    const history = new Float32Array(160);
    let index = 0;
    return onFrame(() => {
      if (!canvas || !open) return;
      history[index] = meterBus.latest.raw_peak_dbfs;
      index = (index + 1) % history.length;
      const context = fitCanvas(canvas);
      if (!context) return;
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      context.clearRect(0, 0, w, h);
      context.fillStyle = cssVar(canvas, '--accent-ink');
      const step = w / history.length;
      for (let i = 0; i < history.length; i++) {
        const db = history[(index + i) % history.length]!;
        const level = Math.min(Math.max((db + 70) / 70, 0), 1);
        const bar = Math.max(1, level * h);
        context.globalAlpha = 0.35 + 0.65 * (i / history.length);
        context.fillRect(i * step, (h - bar) / 2, Math.max(1, step - 1), bar);
      }
      context.globalAlpha = 1;
    });
  });
</script>

<Dialog {open} title={t('calibration.title')} onclose={close} width="520px">
  {#if phase === 'idle'}
    <p class="lead">{t('calibration.intro')}</p>
    <p class="note">{t('mic.doubleProcessing')}</p>
  {:else if phase === 'measuring' && cal.state === 'measuring'}
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
  {:else if phase === 'done' && cal.state === 'done'}
    {@const report = cal.report}
    <div class="stats">
      <div>
        <span class="hud-label">{t('calibration.snrBefore')}</span>
        <span class="big">{report.snr_db.toFixed(1)}<small>dB</small></span>
      </div>
      <div>
        <span class="hud-label">{t('calibration.snrAfter')}</span>
        <span class="big accent">{report.snr_after_db.toFixed(1)}<small>dB</small></span>
      </div>
      <div>
        <span class="hud-label">{t('calibration.noiseLevel')}</span>
        <span class="big">{t(`calibration.noise.${report.noise_level}`)}</span>
      </div>
    </div>
    <span class="hud-label">{t('calibration.changed')}</span>
    <ul class="changes">
      <li style:--c="var(--app-{MODULES.gain.color}-ink)"><span class="dot"></span>{t(MODULES.gain.title)}<span class="value">{formatDb(report.gain_db)}</span></li>
      <li style:--c="var(--app-{MODULES.high_pass.color}-ink)"><span class="dot"></span>{t(MODULES.high_pass.title)}<span class="value">{formatHz(report.high_pass_freq_hz)}</span></li>
      <li style:--c="var(--app-{MODULES.noise_suppression.color}-ink)"><span class="dot"></span>{t(MODULES.noise_suppression.title)}<span class="value">{formatValue(report.noise_intensity_pct, 'percent')}</span></li>
      <li style:--c="var(--app-{MODULES.expander.color}-ink)"><span class="dot"></span>{t(MODULES.expander.title)} · {t('params.threshold')}<span class="value">{formatValue(report.expander_threshold_db, 'dbfs')}</span></li>
    </ul>
    {#if report.suggest_studio}<p class="note">{t('calibration.suggestStudio')}</p>{/if}
  {:else if phase === 'failed' && cal.state === 'failed'}
    <p class="failed">{t(`calibration.failed.${cal.reason}`)}</p>
  {/if}
  {#snippet actions()}
    {#if phase === 'idle'}
      <button class="btn" onclick={close}>{t('calibration.cancel')}</button>
      <button class="btn invert" onclick={start}>{t('calibration.start')}</button>
    {:else if phase === 'measuring'}
      <button class="btn" onclick={close}>{t('calibration.cancel')}</button>
    {:else if phase === 'done'}
      <button class="btn" onclick={onclose}>{t('calibration.fineTune')}</button>
      <button class="btn invert" onclick={onlisten}>{t('calibration.listen')}</button>
    {:else}
      <button class="btn" onclick={onclose}>{t('calibration.close')}</button>
      <button class="btn invert" onclick={start}>{t('calibration.retry')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .lead {
    font-size: 14px;
    color: var(--ink);
  }

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

  .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    margin-bottom: 20px;
  }

  .stats > div {
    display: grid;
    gap: 4px;
    padding: 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .big {
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
  }

  .big small {
    margin-left: 4px;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--muted);
  }

  .big.accent {
    color: var(--app-teal-ink);
  }

  .changes {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: grid;
    gap: 6px;
  }

  .changes li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
    border-bottom: 1px solid var(--line);
  }

  .changes .value {
    margin-left: auto;
  }

  .dot {
    width: 8px;
    height: 8px;
    background: var(--c);
  }

  .failed {
    color: var(--danger);
  }
</style>
