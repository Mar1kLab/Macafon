<script lang="ts">
  import { tip } from './tooltip.svelte';
  // Ruler slider (design system §6): ticks every 10%, major every 50%, fill in the module
  // colour, mono value on the right. Double-click resets, wheel steps (Shift — fine), arrows
  // and PageUp/PageDown move it (PRD §9.8), clicking the value lets you type one.

  interface Props {
    value: number;
    min: number;
    max: number;
    defaultValue: number;
    label: string;
    format: (value: number) => string;
    onchange: (value: number) => void;
    /** Quantisation; 0 rounds to `decimals`. */
    step?: number;
    decimals?: number;
    scale?: 'linear' | 'log';
    /** Value the fill grows from (0 dB for bipolar gains). */
    origin?: number;
    color?: string;
    disabled?: boolean;
    hint?: string;
    layout?: 'stacked' | 'row';
    compact?: boolean;
  }

  let {
    value,
    min,
    max,
    defaultValue,
    label,
    format,
    onchange,
    step = 0,
    decimals = 1,
    scale = 'linear',
    origin,
    color = 'blue',
    disabled = false,
    hint,
    layout = 'stacked',
    compact = false,
  }: Props = $props();

  let track: HTMLDivElement | undefined = $state();
  let editing = $state(false);
  let draft = $state('');
  let dragging = $state(false);

  const toPos = (v: number) =>
    scale === 'log'
      ? Math.log(v / min) / Math.log(max / min)
      : (v - min) / (max - min);
  const fromPos = (p: number) => {
    const clamped = Math.min(Math.max(p, 0), 1);
    return scale === 'log' ? min * (max / min) ** clamped : min + clamped * (max - min);
  };

  function quantize(v: number): number {
    const clamped = Math.min(Math.max(v, min), max);
    if (step > 0) return Math.min(Math.max(Math.round(clamped / step) * step, min), max);
    const factor = 10 ** decimals;
    return Math.round(clamped * factor) / factor;
  }

  function set(v: number) {
    const next = quantize(v);
    if (next !== value) onchange(next);
  }

  /** Moves in slider travel, so log scales step evenly; at least one quantisation step. */
  function nudge(fraction: number) {
    const target = fromPos(toPos(value) + fraction);
    const minimal = step > 0 ? step : 10 ** -decimals;
    if (quantize(target) === value) set(value + Math.sign(fraction) * minimal);
    else set(target);
  }

  const pos = $derived(Math.min(Math.max(toPos(value), 0), 1));
  const originPos = $derived(origin === undefined ? 0 : Math.min(Math.max(toPos(origin), 0), 1));
  const fillLeft = $derived(Math.min(pos, originPos));
  const fillWidth = $derived(Math.abs(pos - originPos));
  const ticks = Array.from({ length: 11 }, (_, i) => i);

  function pointerValue(event: PointerEvent): number {
    const rect = track!.getBoundingClientRect();
    return fromPos((event.clientX - rect.left) / rect.width);
  }

  function onpointerdown(event: PointerEvent) {
    if (disabled || event.button !== 0) return;
    track!.setPointerCapture(event.pointerId);
    dragging = true;
    set(pointerValue(event));
  }

  function onpointermove(event: PointerEvent) {
    if (dragging) set(pointerValue(event));
  }

  function onpointerup(event: PointerEvent) {
    dragging = false;
    track?.releasePointerCapture(event.pointerId);
  }

  function onkeydown(event: KeyboardEvent) {
    if (disabled) return;
    const fine = event.shiftKey ? 0.2 : 1;
    const moves: Record<string, () => void> = {
      ArrowRight: () => nudge(0.01 * fine),
      ArrowUp: () => nudge(0.01 * fine),
      ArrowLeft: () => nudge(-0.01 * fine),
      ArrowDown: () => nudge(-0.01 * fine),
      PageUp: () => nudge(0.1),
      PageDown: () => nudge(-0.1),
      Home: () => set(min),
      End: () => set(max),
    };
    const move = moves[event.key];
    if (move) {
      event.preventDefault();
      move();
    }
  }

  // Wheel needs a non-passive listener to keep the page from scrolling.
  $effect(() => {
    const element = track;
    if (!element) return;
    const onwheel = (event: WheelEvent) => {
      if (disabled) return;
      event.preventDefault();
      const direction = event.deltaY < 0 || event.deltaX > 0 ? 1 : -1;
      nudge(direction * (event.shiftKey ? 0.002 : 0.01));
    };
    element.addEventListener('wheel', onwheel, { passive: false });
    return () => element.removeEventListener('wheel', onwheel);
  });

  function startEdit() {
    if (disabled) return;
    draft = String(quantize(value));
    editing = true;
  }

  function commitEdit() {
    // Accept "−12", "-12 dB", "6.5k".
    const text = draft.replace('−', '-').trim().toLowerCase();
    const match = /^([-+]?\d*[.,]?\d+)\s*(k)?/.exec(text);
    if (match) {
      const parsed = Number(match[1]!.replace(',', '.')) * (match[2] ? 1000 : 1);
      if (Number.isFinite(parsed)) set(parsed);
    }
    editing = false;
  }

  function focusOnMount(element: HTMLInputElement) {
    element.focus();
    element.select();
  }
</script>

<div
  class="ruler {layout}"
  class:disabled
  class:compact
  style:--c="var(--app-{color})"
  style:--c-ink="var(--app-{color}-ink)"
  {@attach tip(hint)}
>
  {#if !compact}<span class="hud-label name">{label}</span>{/if}
  <div
    bind:this={track}
    class="track"
    class:dragging
    role="slider"
    tabindex={disabled ? -1 : 0}
    aria-label={label}
    aria-valuemin={min}
    aria-valuemax={max}
    aria-valuenow={value}
    aria-valuetext={format(value)}
    aria-disabled={disabled}
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={onpointerup}
    ondblclick={() => !disabled && set(defaultValue)}
    {onkeydown}
  >
    <div class="fill" style:left="{fillLeft * 100}%" style:width="{fillWidth * 100}%"></div>
    {#each ticks as tick (tick)}
      <span class="tick" class:major={tick % 5 === 0} style:left="{tick * 10}%"></span>
    {/each}
    <div class="thumb" style:left="{pos * 100}%"></div>
  </div>
  {#if editing}
    <input
      class="value edit"
      bind:value={draft}
      use:focusOnMount
      onblur={commitEdit}
      onkeydown={(event) => {
        if (event.key === 'Enter') commitEdit();
        if (event.key === 'Escape') editing = false;
      }}
      aria-label={label}
    />
  {:else}
    <button class="value readout" onclick={startEdit} tabindex="-1" {disabled}>{format(value)}</button>
  {/if}
</div>

<style>
  .ruler {
    display: grid;
    align-items: center;
    gap: var(--space-2) var(--space-2);
  }

  .ruler.stacked {
    grid-template-columns: 1fr auto;
    grid-template-areas: 'name value' 'track track';
  }

  .ruler.row {
    grid-template-columns: 104px 1fr 78px;
    grid-template-areas: 'name track value';
  }

  /* A narrow card: the name and value go above the track, which keeps its full width. */
  @container (max-width: 300px) {
    .ruler.row {
      grid-template-columns: 1fr auto;
      grid-template-areas: 'name value' 'track track';
      row-gap: var(--space-1);
    }
  }

  .ruler.compact {
    grid-template-columns: 1fr 46px;
    grid-template-areas: 'track value';
  }

  .name {
    grid-area: name;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .track {
    grid-area: track;
    position: relative;
    height: 22px;
    background: var(--bg-sunken);
    border: 1px solid var(--line);
    cursor: ew-resize;
    touch-action: none;
    transition: border-color var(--dur) var(--ease);
  }

  .row .track {
    height: 16px;
  }

  .track:hover,
  .track.dragging {
    border-color: var(--line-strong);
  }

  .fill {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--c);
    opacity: 0.32;
  }

  .tick {
    position: absolute;
    bottom: 0;
    width: 1px;
    height: 5px;
    background: var(--faint);
    transform: translateX(-0.5px);
  }

  .tick.major {
    height: 9px;
    background: var(--line-strong);
  }

  .thumb {
    position: absolute;
    top: -3px;
    bottom: -3px;
    width: 2px;
    background: var(--c-ink);
    transform: translateX(-1px);
  }

  .value {
    grid-area: value;
    justify-self: end;
    text-align: right;
    min-width: 0;
    white-space: nowrap;
  }

  .readout {
    cursor: text;
  }

  .readout:hover {
    color: var(--c-ink);
  }

  .edit {
    width: 78px;
    background: var(--bg-sunken);
    border: 1px solid var(--accent);
    padding: 0 4px;
    outline: none;
    user-select: text;
  }

  .disabled .track {
    cursor: default;
  }

  .disabled .thumb,
  .disabled .fill {
    opacity: 0.25;
  }

  .disabled .readout {
    color: var(--faint);
    cursor: default;
  }
</style>
