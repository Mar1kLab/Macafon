<script lang="ts">
  // TEST panel (§6.4, design system "Transport"): REC with a 3-2-1 countdown (AB-001), LOOP
  // (AB-002), the A ⇄ B switch (AB-004, Space or B), loudness match (AB-005) and the A/B
  // waveforms with click-to-seek (AB-006).
  import { onMount } from 'svelte';
  import { Circle, Headphones, Play, Square } from '@lucide/svelte';
  import Toggle from '../components/Toggle.svelte';
  import { daemon } from '../state';
  import { cssVar, fitCanvas, onFrame } from '../state/frame';
  import { formatMs } from '../dsp/format';
  import { t } from '../i18n';
  import type { AbWaveform } from '../ipc/protocol';

  const RECORD_SECONDS = 10;
  const ab = $derived(daemon.ab);
  let countdown = $state<number | null>(null);
  let waveform = $state.raw<AbWaveform | null>(null);
  let canvas: HTMLCanvasElement | undefined = $state();
  let lastClip: number | null = null;
  let settingsKey = '';

  const recording = $derived(ab?.recording ?? null);
  const hasClip = $derived(ab?.clip_ms != null);
  const side = $derived(ab?.side ?? 'b');

  async function record() {
    if (recording) {
      await daemon.run(daemon.client.call('ab.stop_recording'));
      return;
    }
    for (let n = 3; n > 0; n--) {
      countdown = n;
      await new Promise((resolve) => setTimeout(resolve, 700));
    }
    countdown = null;
    await daemon.run(daemon.client.call('ab.record', { max_seconds: RECORD_SECONDS }));
  }

  function loop() {
    if (!ab) return;
    void daemon.run(ab.playing ? daemon.client.call('ab.stop') : daemon.client.call('ab.play', { device: null }));
  }

  function toggleSide() {
    void daemon.run(daemon.client.call('ab.toggle'));
  }

  async function loadWaveform() {
    const points = Math.max(16, Math.min(4096, Math.round((canvas?.clientWidth ?? 600) / 2)));
    const result = await daemon.run(daemon.client.call('ab.waveform', { points }));
    if (result) waveform = result;
  }

  // New clip → new waveform; chain changes reshape B (debounced).
  $effect(() => {
    const clip = ab?.clip_ms ?? null;
    if (clip !== lastClip) {
      lastClip = clip;
      if (clip !== null) void loadWaveform();
      else waveform = null;
    }
  });

  $effect(() => {
    const key = JSON.stringify(daemon.mic?.settings ?? null);
    if (key === settingsKey || !hasClip) {
      settingsKey = key;
      return;
    }
    settingsKey = key;
    const timer = setTimeout(loadWaveform, 400);
    return () => clearTimeout(timer);
  });

  onMount(() => {
    const onkey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      if (target.closest('input, textarea, [role="slider"], [role="listbox"], dialog') || event.ctrlKey || event.metaKey) return;
      // Space keeps activating a focused button; B works everywhere.
      if (event.key === ' ' && target.closest('button, a')) return;
      if ((event.key === ' ' || event.key.toLowerCase() === 'b') && hasClip) {
        event.preventDefault();
        toggleSide();
      }
    };
    window.addEventListener('keydown', onkey);
    const stop = onFrame(() => drawWaveform());
    return () => {
      window.removeEventListener('keydown', onkey);
      stop();
    };
  });

  function drawWaveform() {
    if (!canvas) return;
    const context = fitCanvas(canvas);
    if (!context) return;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    context.clearRect(0, 0, w, h);
    const mid = h / 2;
    const line = cssVar(canvas, '--line');
    context.fillStyle = line;
    context.fillRect(0, Math.round(mid), w, 1);
    const state = daemon.ab;
    if (state?.recording) {
      const progress = state.recording.elapsed_ms / state.recording.max_ms;
      context.fillStyle = cssVar(canvas, '--danger');
      context.globalAlpha = 0.5;
      context.fillRect(0, mid - 1, progress * w, 2);
      context.globalAlpha = 1;
      return;
    }
    if (!waveform || !state?.clip_ms) return;
    const draw = (values: number[], color: string, alpha: number) => {
      context.fillStyle = color;
      context.globalAlpha = alpha;
      const step = w / values.length;
      values.forEach((value, i) => {
        const half = Math.max(0.5, value * (mid - 2));
        context.fillRect(i * step, mid - half, Math.max(1, step - 1), half * 2);
      });
    };
    const a = cssVar(canvas, '--muted');
    const b = cssVar(canvas, '--accent-ink');
    // The selected side is drawn on top at full strength.
    if (state.side === 'a') {
      draw(waveform.b, b, 0.25);
      draw(waveform.a, a, 0.8);
    } else {
      draw(waveform.a, a, 0.25);
      draw(waveform.b, b, 0.8);
    }
    context.globalAlpha = 1;
    const x = (state.position_ms / state.clip_ms) * w;
    context.fillStyle = cssVar(canvas, '--ink-strong');
    context.fillRect(Math.round(x), 0, 1, h);
  }

  function seek(event: MouseEvent) {
    if (!ab?.clip_ms || !canvas) return;
    const rect = canvas.getBoundingClientRect();
    const position_ms = Math.round(((event.clientX - rect.left) / rect.width) * ab.clip_ms);
    void daemon.run(daemon.client.call('ab.seek', { position_ms }));
  }
</script>

<section class="test" aria-label={t('test.title')}>
  <span class="hud-label title">{t('test.title')}</span>
  <button
    class="btn rec"
    class:recording={recording !== null || countdown !== null}
    onclick={record}
    disabled={countdown !== null || (recording !== null && recording.elapsed_ms < 3000)}
    aria-label={recording ? t('test.stop') : t('test.rec')}
  >
    {#if recording}
      <Square size={10} fill="currentColor" strokeWidth={0} />
      {formatMs(recording.elapsed_ms)}
    {:else if countdown !== null}
      {countdown}
    {:else}
      <Circle size={10} fill="currentColor" strokeWidth={0} />
      {t('test.rec')} {RECORD_SECONDS}s
    {/if}
  </button>
  <button class="btn" class:on={ab?.playing} onclick={loop} disabled={!hasClip || recording !== null}>
    {#if ab?.playing}<Square size={10} fill="currentColor" strokeWidth={0} />{t('test.stop')}{:else}<Play size={11} fill="currentColor" strokeWidth={0} />{t('test.loop')}{/if}
  </button>
  <div class="ab" role="radiogroup" aria-label="A / B" title={t('test.toggleHint')}>
    <button role="radio" aria-checked={side === 'a'} class:selected={side === 'a'} disabled={!hasClip} onclick={() => daemon.run(daemon.client.call('ab.select', { side: 'a' }))}>
      <b>A</b> {t('test.a')}
    </button>
    <span class="swap" aria-hidden="true">⇄</span>
    <button role="radio" aria-checked={side === 'b'} class:selected={side === 'b'} disabled={!hasClip} onclick={() => daemon.run(daemon.client.call('ab.select', { side: 'b' }))}>
      <b>B</b> {t('test.b')}
    </button>
  </div>
  <div class="wave">
    <canvas bind:this={canvas} onclick={seek} aria-label="A/B waveform"></canvas>
    {#if countdown !== null || recording}
      <span class="overlay hud-label">{countdown !== null ? `${t('test.countdown')} · ${countdown}` : t('test.prompt')}</span>
    {:else if !hasClip}
      <span class="overlay hint">{t('test.empty')}</span>
    {/if}
  </div>
  <label class="match" title={t('test.loudnessMatch')}>
    <span class="hud-label">{t('test.loudnessMatch')}</span>
    <Toggle
      checked={ab?.loudness_match ?? true}
      label={t('test.loudnessMatch')}
      disabled={!ab}
      onchange={(enabled) => daemon.run(daemon.client.call('ab.set_loudness_match', { enabled }))}
    />
  </label>
  <span class="headphones" title={t('test.headphones')}><Headphones size={14} strokeWidth={1.5} /></span>
</section>

<style>
  .test {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 64px;
    padding: 0 24px;
    border-top: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .title {
    color: var(--ink);
    margin-right: 6px;
  }

  .rec {
    min-width: 96px;
    justify-content: center;
    color: var(--danger);
  }

  .rec.recording {
    border-color: var(--danger);
  }

  .ab {
    display: flex;
    align-items: center;
    border: 1px solid var(--line-strong);
    height: 34px;
  }

  .ab button {
    height: 100%;
    padding: 0 14px;
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--muted);
    transition:
      background var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  .ab b {
    font-family: var(--font-display);
    font-size: 15px;
    letter-spacing: 0;
    margin-right: 6px;
  }

  .ab button.selected {
    background: var(--invert-bg);
    color: var(--invert-fg);
  }

  .ab button:disabled:not(.selected) {
    color: var(--faint);
  }

  .ab button.selected:disabled {
    opacity: 0.5;
  }

  .swap {
    padding: 0 6px;
    color: var(--faint);
  }

  .wave {
    position: relative;
    flex: 1;
    min-width: 120px;
    height: 40px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  canvas {
    width: 100%;
    height: 100%;
    display: block;
    cursor: pointer;
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
  }

  .hint {
    font-size: 12px;
    color: var(--faint);
  }

  .match {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .headphones {
    color: var(--faint);
    display: inline-flex;
  }
</style>
