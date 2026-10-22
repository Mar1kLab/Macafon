<script lang="ts">
  import { t } from '../i18n';

  interface Props {
    value: number;
    label: string;
    color: string;
    muted?: boolean;
    /** Sketch: the app's ruler with sharp corners instead of the rounded pill. */
    sketch?: boolean;
    onchange: (value: number) => void;
  }

  let { value, label, color, muted = false, sketch = false, onchange }: Props = $props();
  let track: HTMLDivElement | undefined = $state();
  let pointer: number | null = $state(null);
  const clamp = (next: number) => Math.min(100, Math.max(0, Math.round(next)));
  const percent = $derived(clamp(value));

  function fromPointer(event: PointerEvent) {
    if (!track) return;
    const rect = track.getBoundingClientRect();
    onchange(clamp((1 - (event.clientY - rect.top) / rect.height) * 100));
  }

  function onkeydown(event: KeyboardEvent) {
    let next: number;
    switch (event.key) {
      case 'ArrowUp': next = percent + 5; break;
      case 'ArrowDown': next = percent - 5; break;
      case 'PageUp': next = percent + 10; break;
      case 'PageDown': next = percent - 10; break;
      case 'Home': next = 0; break;
      case 'End': next = 100; break;
      default: return;
    }
    event.preventDefault();
    onchange(clamp(next));
  }
</script>

<div class="fader" class:muted class:sketch style:--color="var(--app-{color})" style:--color-ink="var(--app-{color}-ink)">
  <span class="readout" aria-hidden="true">{t('quick.volumeValue', { value: percent })}</span>
  <div
    class="track"
    class:dragging={pointer !== null}
    bind:this={track}
    role="slider"
    tabindex="0"
    aria-label={label}
    aria-orientation="vertical"
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={percent}
    aria-valuetext={t(muted ? 'quick.mutedVolumeValue' : 'quick.volumeValue', { value: percent })}
    style:--fill="{percent}%"
    {onkeydown}
    onwheel={(event) => {
      event.preventDefault();
      if (event.deltaY !== 0) onchange(clamp(percent + (event.deltaY < 0 ? 5 : -5)));
    }}
    onpointerdown={(event) => {
      if (event.button !== 0 || pointer !== null) return;
      event.currentTarget.focus();
      event.currentTarget.setPointerCapture(event.pointerId);
      pointer = event.pointerId;
      fromPointer(event);
    }}
    onpointermove={(event) => event.pointerId === pointer && fromPointer(event)}
    onpointerup={(event) => {
      if (event.pointerId !== pointer) return;
      pointer = null;
      event.currentTarget.releasePointerCapture(event.pointerId);
    }}
    onpointercancel={() => (pointer = null)}
    onlostpointercapture={() => (pointer = null)}
  >
    <span class="fill"></span>
    {#if sketch}
      {#each [10, 20, 30, 40, 50, 60, 70, 80, 90] as tick (tick)}
        <span class="tick" class:major={tick === 50} style:bottom="{tick}%"></span>
      {/each}
    {:else}
      <span class="ticks"></span>
    {/if}
    <span class="thumb"></span>
  </div>
  <span class="state" class:quiet={!muted} aria-hidden="true">{muted ? t('quick.muted') : t('quick.on')}</span>
</div>

<style>
  .fader { display: flex; flex-direction: column; align-items: center; gap: var(--space-2); }
  .readout { font-family: var(--font-mono); font-size: var(--text-md); color: var(--color-ink); font-variant-numeric: tabular-nums; }
  .track { position: relative; width: 44px; height: 144px; border: 1px solid var(--line); border-radius: 22px; background: var(--bg-sunken); cursor: ns-resize; touch-action: none; user-select: none; }
  .fill { position: absolute; inset: auto 0 0; height: var(--fill); background: var(--color); border-radius: 22px; transition: height var(--dur) var(--ease), opacity var(--dur) var(--ease); }
  .ticks { position: absolute; inset: 12px 15px; background: repeating-linear-gradient(to top, var(--line-strong) 0 1px, transparent 1px 12px); pointer-events: none; }
  .thumb { position: absolute; left: 8px; right: 8px; bottom: clamp(8px, var(--fill), 134px); height: 3px; background: var(--ink-strong); border-radius: 2px; pointer-events: none; transition: bottom var(--dur) var(--ease); }
  .track:focus-visible { outline: 2px solid var(--color-ink); outline-offset: 4px; }
  .track:hover { border-color: var(--color-ink); }
  .state { color: var(--danger); font-size: var(--text-2xs); font-family: var(--font-mono); }
  .quiet { color: var(--faint); }
  .muted .fill { opacity: 0.25; }
  .muted .readout { color: var(--muted); }
  .dragging .fill, .dragging .thumb { transition: none; }
  /* Sketch: a ruler — sharp corners, ticks every 10 % (major at 50 %), a straight head line. */
  .sketch .track { width: 36px; border-radius: 0; }
  .sketch .fill { border-radius: 0; opacity: 0.85; }
  .sketch .tick { position: absolute; left: 0; width: 8px; height: 1px; background: var(--faint); pointer-events: none; }
  .sketch .tick.major { width: 14px; background: var(--line-strong); }
  .sketch .thumb { left: -1px; right: -1px; bottom: calc(var(--fill) - 1px); height: 2px; border-radius: 0; }
  .sketch .readout, .sketch .state { letter-spacing: 0.1em; text-transform: uppercase; }
  .sketch .track:focus-visible { outline: 1px solid var(--accent); outline-offset: 2px; }
  .muted.sketch .fill { opacity: 0.2; }
  @media (prefers-reduced-motion: reduce) { .fill, .thumb { transition: none; } }
</style>
