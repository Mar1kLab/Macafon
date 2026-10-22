<script lang="ts">
  // EQ graph (EQ-001): log frequency grid, total response 2px with 8% fill, draggable band
  // points (X — frequency, Y — gain, wheel or Alt+drag — Q), double-click adds a band,
  // right-click opens type/delete. Points are focusable for keyboard editing (PRD §9.8).
  import { onMount } from 'svelte';
  import { cssVar, fitCanvas } from '../state/frame';
  import { GRAPH_MAX_HZ, GRAPH_MIN_HZ, eqGainDb, freqToX, xToFreq } from '../dsp/eq-response';
  import { formatDb, formatHz } from '../dsp/format';
  import { EQ_MAX_BANDS, SPECS } from '../ipc/dsp';
  import type { EqBand, EqFilter } from '../ipc/protocol';
  import { t } from '../i18n';
  import { tip } from './tooltip.svelte';
  import Menu, { type MenuAnchor, type MenuEntry } from './Menu.svelte';

  interface Props {
    bands: EqBand[];
    outputGainDb: number;
    color: string;
    selected: number | null;
    disabled?: boolean;
    height?: number;
    onselect: (index: number | null) => void;
    onchange: (index: number, band: EqBand) => void;
    onadd: (band: EqBand) => void;
    ondelete: (index: number) => void;
  }

  let {
    bands,
    outputGainDb,
    color,
    selected,
    disabled = false,
    height = 150,
    onselect,
    onchange,
    onadd,
    ondelete,
  }: Props = $props();

  const RANGE_DB = 18;
  const FREQ_LINES = [50, 100, 200, 500, 1000, 2000, 5000, 10_000];
  const FREQ_LABELS: Record<number, string> = { 100: '100', 1000: '1k', 10_000: '10k' };
  const FILTERS: EqFilter[] = ['bell', 'low_shelf', 'high_shelf', 'low_cut', 'high_cut', 'notch'];

  let canvas: HTMLCanvasElement | undefined = $state();
  let box: HTMLDivElement | undefined = $state();
  let width = $state(0);
  let menu = $state<{ index: number; anchor: MenuAnchor; point: HTMLElement } | null>(null);
  let drag: { index: number; pointer: number; startY: number; startQ: number; q: boolean } | null = null;

  const hasGain = (filter: EqFilter) => filter === 'bell' || filter === 'low_shelf' || filter === 'high_shelf';
  const gainToY = (db: number) => ((RANGE_DB - db) / (2 * RANGE_DB)) * height;
  const yToGain = (y: number) => RANGE_DB - (y / height) * 2 * RANGE_DB;
  const clamp = (v: number, min: number, max: number) => Math.min(Math.max(v, min), max);
  const round = (v: number, digits: number) => Math.round(v * 10 ** digits) / 10 ** digits;

  function pointY(band: EqBand): number {
    return gainToY(hasGain(band.filter) ? band.gain_db : 0);
  }

  function draw() {
    if (!canvas) return;
    const context = fitCanvas(canvas);
    if (!context) return;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    context.clearRect(0, 0, w, h);
    const line = cssVar(canvas, '--line');
    const faint = cssVar(canvas, '--faint');
    const ink = cssVar(canvas, `--app-${color}-ink`);
    context.lineWidth = 1;
    context.strokeStyle = line;
    context.fillStyle = faint;
    context.font = `${cssVar(canvas, '--text-2xs')} ${cssVar(canvas, '--font-mono')}`;
    for (const freq of FREQ_LINES) {
      const x = Math.round(freqToX(freq) * w) + 0.5;
      context.beginPath();
      context.moveTo(x, 0);
      context.lineTo(x, h);
      context.stroke();
      const label = FREQ_LABELS[freq];
      if (label) context.fillText(label, x + 3, h - 4);
    }
    for (let db = -12; db <= 12; db += 6) {
      const y = Math.round(gainToY(db)) + 0.5;
      context.globalAlpha = db === 0 ? 1 : 0.5;
      context.beginPath();
      context.moveTo(0, y);
      context.lineTo(w, y);
      context.stroke();
    }
    context.globalAlpha = 1;
    // Total response, one point per two pixels.
    const zero = gainToY(0);
    const points: [number, number][] = [];
    for (let x = 0; x <= w; x += 2) {
      const freq = xToFreq(x / w);
      const y = clamp(gainToY(eqGainDb(bands, outputGainDb, freq)), -2, h + 2);
      points.push([x, y]);
    }
    context.beginPath();
    context.moveTo(0, zero);
    points.forEach(([x, y]) => context.lineTo(x, y));
    context.lineTo(w, zero);
    context.closePath();
    context.fillStyle = ink;
    context.globalAlpha = disabled ? 0.03 : 0.08;
    context.fill();
    context.globalAlpha = disabled ? 0.4 : 1;
    context.beginPath();
    points.forEach(([x, y], i) => (i === 0 ? context.moveTo(x, y) : context.lineTo(x, y)));
    context.strokeStyle = ink;
    context.lineWidth = 2;
    context.stroke();
    context.globalAlpha = 1;
  }

  onMount(() => {
    const observer = new ResizeObserver(() => {
      width = box?.clientWidth ?? 0;
      draw();
    });
    observer.observe(box!);
    return () => observer.disconnect();
  });

  $effect(() => {
    // Redraw whenever the bands or the size change.
    void [bands, outputGainDb, height, width, disabled, color];
    draw();
  });

  function bandFromPointer(index: number, event: PointerEvent): EqBand {
    const band = bands[index]!;
    const rect = box!.getBoundingClientRect();
    if (drag?.q) {
      const factor = 2 ** ((drag.startY - event.clientY) / 60);
      return { ...band, q: round(clamp(drag.startQ * factor, SPECS.eq_q.min, SPECS.eq_q.max), 2) };
    }
    const freq = clamp(xToFreq((event.clientX - rect.left) / rect.width), SPECS.eq_freq_hz.min, SPECS.eq_freq_hz.max);
    const gain = hasGain(band.filter)
      ? clamp(round(yToGain(event.clientY - rect.top), 1), SPECS.eq_gain_db.min, SPECS.eq_gain_db.max)
      : band.gain_db;
    return { ...band, freq_hz: Math.round(freq), gain_db: gain };
  }

  function onpointstart(index: number, event: PointerEvent) {
    if (disabled || event.button !== 0) return;
    event.stopPropagation();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    drag = { index, pointer: event.pointerId, startY: event.clientY, startQ: bands[index]!.q, q: event.altKey };
    onselect(index);
  }

  function onpointmove(index: number, event: PointerEvent) {
    if (drag?.index === index && drag.pointer === event.pointerId) onchange(index, bandFromPointer(index, event));
  }

  function onpointend() {
    drag = null;
  }

  function wheelQ(index: number, event: WheelEvent) {
    if (disabled) return;
    event.preventDefault();
    const band = bands[index]!;
    const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1;
    onchange(index, { ...band, q: round(clamp(band.q * factor, SPECS.eq_q.min, SPECS.eq_q.max), 2) });
  }

  function onkeydown(index: number, event: KeyboardEvent) {
    if (disabled) return;
    const band = bands[index]!;
    const fine = event.shiftKey ? 0.25 : 1;
    let next: EqBand | null = null;
    switch (event.key) {
      case 'ArrowLeft':
      case 'ArrowRight': {
        const octave = (event.key === 'ArrowRight' ? 1 : -1) * fine / 12;
        next = { ...band, freq_hz: Math.round(clamp(band.freq_hz * 2 ** octave, SPECS.eq_freq_hz.min, SPECS.eq_freq_hz.max)) };
        break;
      }
      case 'ArrowUp':
      case 'ArrowDown':
        if (hasGain(band.filter)) {
          const step = (event.key === 'ArrowUp' ? 0.5 : -0.5) * fine;
          next = { ...band, gain_db: round(clamp(band.gain_db + step, SPECS.eq_gain_db.min, SPECS.eq_gain_db.max), 2) };
        }
        break;
      case '+':
      case '=':
      case '-': {
        const factor = event.key === '-' ? 1 / 1.1 : 1.1;
        next = { ...band, q: round(clamp(band.q * factor, SPECS.eq_q.min, SPECS.eq_q.max), 2) };
        break;
      }
      case 'Delete':
      case 'Backspace':
        event.preventDefault();
        ondelete(index);
        return;
      case 'ContextMenu':
        openMenu(index, event.currentTarget as HTMLElement);
        return;
    }
    if (next) {
      event.preventDefault();
      onchange(index, next);
    }
  }

  function openMenu(index: number, point: HTMLElement, at?: { x: number; y: number }) {
    menu = { index, anchor: at ?? point, point };
    onselect(index);
  }

  /** The band's filter type and Delete (EQ-006). */
  const menuEntries = $derived.by((): MenuEntry[] => {
    const band = menu ? bands[menu.index] : undefined;
    if (!menu || !band) return [];
    const index = menu.index;
    return [
      { kind: 'heading', label: t('eq.band', { n: index + 1 }) },
      { kind: 'radio', value: band.filter, options: FILTERS.map((filter) => ({ value: filter, label: t(`eq.filters.${filter}`) })), onselect: (filter) => onchange(index, { ...band, filter: filter as EqFilter }) },
      { kind: 'separator' },
      { kind: 'item', label: t('eq.deleteBand'), danger: true, onselect: () => ondelete(index) },
    ];
  });

  function closeMenu() {
    const point = menu?.point;
    menu = null;
    queueMicrotask(() => {
      if (document.activeElement === document.body && point?.isConnected) point.focus();
    });
  }

  function ondblclick(event: MouseEvent) {
    if (disabled || bands.length >= EQ_MAX_BANDS || event.target !== canvas) return;
    const rect = box!.getBoundingClientRect();
    const freq = clamp(xToFreq((event.clientX - rect.left) / rect.width), GRAPH_MIN_HZ, GRAPH_MAX_HZ);
    const gain = clamp(round(yToGain(event.clientY - rect.top), 1), SPECS.eq_gain_db.min, SPECS.eq_gain_db.max);
    onadd({ filter: 'bell', freq_hz: Math.round(freq), gain_db: gain, q: 1 });
  }

  function wheelAction(node: HTMLElement, index: number) {
    let current = index;
    const handler = (event: WheelEvent) => wheelQ(current, event);
    node.addEventListener('wheel', handler, { passive: false });
    return {
      update(next: number) {
        current = next;
      },
      destroy() {
        node.removeEventListener('wheel', handler);
      },
    };
  }
</script>

<div
  class="graph"
  class:disabled
  bind:this={box}
  style:height="{height}px"
  style:--c="var(--app-{color})"
  style:--c-ink="var(--app-{color}-ink)"
  {@attach tip(t('eq.graphHelp'))}
  role="group"
  aria-label={t('eq.graphLabel')}
>
  <canvas bind:this={canvas} aria-hidden="true" {ondblclick}></canvas>
  {#each bands as band, index (index)}
    <button
      class="point"
      class:selected={selected === index}
      style:left="{freqToX(band.freq_hz) * 100}%"
      style:top="{pointY(band)}px"
      aria-roledescription="EQ band"
      aria-label="{t('eq.band', { n: index + 1 })}: {t(`eq.filters.${band.filter}`)}, {formatHz(band.freq_hz)}{hasGain(band.filter) ? `, ${formatDb(band.gain_db)}` : ''}, Q {band.q.toFixed(2)}"
      tabindex={disabled ? -1 : 0}
      onpointerdown={(event) => onpointstart(index, event)}
      onpointermove={(event) => onpointmove(index, event)}
      onpointerup={onpointend}
      onpointercancel={onpointend}
      onfocus={() => onselect(index)}
      onkeydown={(event) => onkeydown(index, event)}
      oncontextmenu={(event) => {
        event.preventDefault();
        if (!disabled) openMenu(index, event.currentTarget, { x: event.clientX, y: event.clientY });
      }}
      use:wheelAction={index}
    >
      <span class="label">{index + 1}</span>
    </button>
  {/each}
</div>
<Menu anchor={menu?.anchor ?? null} label={t('eq.bandMenu')} entries={menuEntries} onclose={closeMenu} />

<style>
  .graph {
    position: relative;
    background: var(--bg-sunken);
    border: 1px solid var(--line);
  }

  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }

  /* EQ points are the one round element of the design system (§4). */
  .point {
    position: absolute;
    width: 14px;
    height: 14px;
    margin: -7px 0 0 -7px;
    border-radius: 50%;
    border: 1px solid var(--c-ink);
    background: color-mix(in srgb, var(--c) 45%, var(--bg-sunken));
    cursor: grab;
    touch-action: none;
    transition: transform var(--dur) var(--ease);
  }

  .point:hover {
    transform: scale(1.15);
  }

  .point.selected {
    background: var(--c-ink);
  }

  .point:focus-visible {
    outline-offset: 3px;
  }

  .label {
    position: absolute;
    left: 50%;
    top: -15px;
    transform: translateX(-50%);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    color: var(--muted);
    pointer-events: none;
  }

  .disabled .point {
    opacity: 0.35;
    cursor: default;
  }





</style>
