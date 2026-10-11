<script lang="ts">
  // Vertical channel fader (design system §6, CH-004): ruler ticks every 10%, fill in the
  // channel colour, cubic volume curve shown in % and dB. Arrows step 1%, PageUp/PageDown 10%,
  // Home/End to the ends, double-click resets, wheel steps (PRD §9.8).
  import { formatVolumeDb } from '../dsp/format';

  interface Props {
    value: number;
    label: string;
    color?: string;
    defaultValue?: number;
    disabled?: boolean;
    onchange: (value: number) => void;
  }

  let { value, label, color = 'blue', defaultValue = 100, disabled = false, onchange }: Props = $props();
  let track: HTMLDivElement | undefined = $state();
  let dragging = $state(false);

  const clamp = (v: number) => Math.round(Math.min(Math.max(v, 0), 100));

  function set(v: number) {
    const next = clamp(v);
    if (next !== value) onchange(next);
  }

  function fromPointer(event: PointerEvent) {
    if (!track) return;
    const rect = track.getBoundingClientRect();
    set(((rect.bottom - event.clientY) / rect.height) * 100);
  }

  function onpointerdown(event: PointerEvent) {
    if (disabled || event.button !== 0) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    dragging = true;
    fromPointer(event);
  }

  function onkeydown(event: KeyboardEvent) {
    const steps: Record<string, number> = { ArrowUp: 1, ArrowRight: 1, ArrowDown: -1, ArrowLeft: -1, PageUp: 10, PageDown: -10 };
    if (event.key in steps) set(value + steps[event.key]!);
    else if (event.key === 'Home') set(0);
    else if (event.key === 'End') set(100);
    else return;
    event.preventDefault();
  }

  const text = $derived(`${Math.round(value)}% · ${formatVolumeDb(value)}`);
</script>

<div
  class="fader"
  class:dragging
  class:disabled
  role="slider"
  tabindex={disabled ? -1 : 0}
  aria-label={label}
  aria-orientation="vertical"
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={Math.round(value)}
  aria-valuetext={text}
  aria-disabled={disabled}
  title={text}
  style:--c="var(--app-{color})"
  style:--c-ink="var(--app-{color}-ink)"
  {onkeydown}
  {onpointerdown}
  onpointermove={(event) => dragging && fromPointer(event)}
  onpointerup={() => (dragging = false)}
  onpointercancel={() => (dragging = false)}
  ondblclick={() => !disabled && set(defaultValue)}
  onwheel={(event) => {
    if (disabled) return;
    event.preventDefault();
    set(value + (event.deltaY < 0 ? 1 : -1) * (event.shiftKey ? 1 : 5));
  }}
>
  <div class="track" bind:this={track}>
    <div class="fill" style:height="{value}%"></div>
    {#each [10, 20, 30, 40, 50, 60, 70, 80, 90] as tick (tick)}
      <span class="tick" class:major={tick === 50} style:bottom="{tick}%"></span>
    {/each}
    <span class="thumb" style:bottom="{value}%"></span>
  </div>
</div>

<style>
  .fader {
    height: 100%;
    width: 28px;
    display: flex;
    justify-content: center;
    cursor: ns-resize;
    touch-action: none;
  }

  .fader:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 2px;
  }

  .disabled {
    opacity: 0.4;
    cursor: default;
  }

  .track {
    position: relative;
    width: 10px;
    height: 100%;
    background: var(--bg-sunken);
    border: 1px solid var(--line);
  }

  .fill {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    background: color-mix(in srgb, var(--c) 45%, transparent);
  }

  .tick {
    position: absolute;
    left: -5px;
    width: 4px;
    height: 1px;
    background: var(--line-strong);
  }

  .tick.major {
    width: 6px;
    left: -7px;
  }

  .thumb {
    position: absolute;
    left: -9px;
    width: 26px;
    height: 3px;
    margin-bottom: -1px;
    background: var(--c-ink);
    transition: transform var(--dur) var(--ease);
  }

  .fader:hover .thumb,
  .dragging .thumb {
    transform: scaleY(1.6);
  }
</style>
