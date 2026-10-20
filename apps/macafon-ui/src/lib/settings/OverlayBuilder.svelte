<script lang="ts">
  // The overlay's layout editor (§9.9): a small copy of the overlay with its parts where they
  // will be, and a tray of hidden ones. Drag a part to move it, into the tray to hide it, out of
  // the tray to show it; a click shows or hides it too. Volumes goes left or right of the
  // controls. Keyboard: arrows move the focused part, Enter or Space shows or hides it.
  import { AudioLines, AudioWaveform, ChevronLeft, ChevronRight, Gauge, History, GripVertical, Headphones, Layers, Mic, SlidersHorizontal, SlidersVertical, Speech, RotateCcw } from '@lucide/svelte';
  import type { Component } from 'svelte';
  import type { OverlayItem, OverlayPrefs } from '../ipc/protocol';
  import { daemon } from '../state';
  import { toCss } from '../components/zoom';
  import { OVERLAY_PARTS, isSwitch, movePart, overlayOrder, overlayRows, partShown, volumesFirst } from '../quick/layout';
  import { t } from '../i18n';

  let { setup, onchange }: { setup: OverlayPrefs; onchange: (patch: Partial<OverlayPrefs>) => void } = $props();

  const ICONS: Record<OverlayItem, Component> = {
    volumes: SlidersVertical,
    mic_switch: Mic,
    processing_switch: AudioLines,
    mixer_switch: SlidersHorizontal,
    dictation: Speech,
    dictation_history: History,
    sounds: AudioWaveform,
    output_device: Headphones,
    mic_level: Gauge,
    preset: Layers,
  };

  const extra = $derived({
    dictation: daemon.dictation ? daemon.dictation.prefs.show_in_overlay : null,
    sounds: daemon.soundpad ? daemon.soundpad.prefs.show_in_overlay : null,
  });
  const available = (part: OverlayItem) => ((part === 'dictation' || part === 'dictation_history') ? extra.dictation !== null : part === 'sounds' ? extra.sounds !== null : true);
  const order = $derived(overlayOrder(setup.order).filter(available));
  const shown = $derived(order.filter((part) => partShown(setup, part, extra)));
  const hidden = $derived(order.filter((part) => !partShown(setup, part, extra)));
  const rows = $derived(overlayRows(shown));
  const volumesLeft = $derived(volumesFirst(shown));
  const strips = $derived([t('mixer.master'), ...(daemon.mixer?.channels.filter((channel) => channel.enabled).map((channel) => channel.name) ?? [])].slice(0, 4));
  const isDefault = $derived(overlayOrder(setup.order).every((part, index) => part === OVERLAY_PARTS[index]));

  function setShown(part: OverlayItem, on: boolean, patch: Partial<OverlayPrefs> = {}) {
    if (part === 'dictation') daemon.setDictationPrefs({ show_in_overlay: on });
    else if (part === 'sounds') daemon.setSoundpadPrefs({ show_in_overlay: on });
    else patch = { ...patch, [part]: on };
    if (Object.keys(patch).length) onchange(patch);
  }

  /** The full saved order (with parts this platform lacks), `part` moved next to `target`. */
  const reorder = (part: OverlayItem, target: OverlayItem | 'start' | 'end', after = false) => movePart(overlayOrder(setup.order), part, target, after);

  // Dragging: a ghost follows the pointer and a mark shows where the part lands.
  type Target = { zone: 'tray' } | { zone: 'canvas'; ref: OverlayItem | 'start' | 'end'; after: boolean; axis: 'x' | 'y' };
  interface Drag {
    part: OverlayItem;
    from: 'canvas' | 'tray';
    pointer: number;
    startX: number;
    startY: number;
    x: number;
    y: number;
    dx: number;
    dy: number;
    width: number;
    height: number;
    moved: boolean;
  }
  let drag = $state<Drag | null>(null);
  let target = $state<Target | null>(null);
  let canvas: HTMLDivElement | undefined = $state();
  let tray: HTMLDivElement | undefined = $state();

  $effect(() => {
    if (!drag?.moved) return;
    document.documentElement.classList.add('overlay-builder-dragging');
    return () => document.documentElement.classList.remove('overlay-builder-dragging');
  });

  const inside = (rect: DOMRect, x: number, y: number) => x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;

  function locate(x: number, y: number, part: OverlayItem): Target | null {
    if (tray && inside(tray.getBoundingClientRect(), x, y)) return { zone: 'tray' };
    if (!canvas) return null;
    const area = canvas.getBoundingClientRect();
    if (!inside(area, x, y)) return null;
    if (part === 'volumes') {
      const after = x > area.left + area.width / 2;
      return { zone: 'canvas', ref: after ? 'end' : 'start', after, axis: 'x' };
    }
    const chips = [...canvas.querySelectorAll<HTMLElement>('[data-control]')].filter((chip) => chip.dataset.control !== part);
    if (chips.length === 0) {
      // An empty column: keep Volumes on its side.
      const volumesRight = shown.includes('volumes') && !volumesLeft;
      return { zone: 'canvas', ref: volumesRight ? 'volumes' : 'end', after: false, axis: 'y' };
    }
    let best: { chip: HTMLElement; rect: DOMRect; distance: number } | null = null;
    for (const chip of chips) {
      const rect = chip.getBoundingClientRect();
      const distance = Math.hypot(Math.max(rect.left - x, 0, x - rect.right), Math.max(rect.top - y, 0, y - rect.bottom));
      if (!best || distance < best.distance) best = { chip, rect, distance };
    }
    const ref = best!.chip.dataset.control as OverlayItem;
    const axis = isSwitch(ref) && isSwitch(part) ? 'x' : 'y';
    const after = axis === 'x' ? x > best!.rect.left + best!.rect.width / 2 : y > best!.rect.top + best!.rect.height / 2;
    return { zone: 'canvas', ref, after, axis };
  }

  function onpointerdown(event: PointerEvent, part: OverlayItem, from: 'canvas' | 'tray') {
    if (event.button !== 0) return;
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    drag = {
      part,
      from,
      pointer: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      x: event.clientX,
      y: event.clientY,
      dx: event.clientX - rect.left,
      dy: event.clientY - rect.top,
      width: rect.width,
      height: rect.height,
      moved: false,
    };
  }

  function onpointermove(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointer) return;
    drag.x = event.clientX;
    drag.y = event.clientY;
    if (!drag.moved && Math.hypot(drag.x - drag.startX, drag.y - drag.startY) > 4) drag.moved = true;
    if (drag.moved) target = locate(drag.x, drag.y, drag.part);
  }

  function onpointerup(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointer) return;
    const { part, from, moved } = drag;
    const drop = moved ? locate(event.clientX, event.clientY, part) : null;
    drag = null;
    target = null;
    if (!moved) return setShown(part, from === 'tray');
    if (!drop) return;
    if (drop.zone === 'tray') {
      if (from === 'canvas') setShown(part, false);
      return;
    }
    const next = reorder(part, drop.ref, drop.after);
    if (from === 'tray') setShown(part, true, { order: next });
    else onchange({ order: next });
  }

  function oncancel() {
    drag = null;
    target = null;
  }

  /** Arrows step a part through the shown (or hidden) ones; Enter and Space show or hide it. */
  function onkeydown(event: KeyboardEvent, part: OverlayItem, from: 'canvas' | 'tray') {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      setShown(part, from === 'tray');
      requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-builder-part="${part}"]`)?.focus());
      return;
    }
    const step = event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : 0;
    if (!step) return;
    event.preventDefault();
    const list = from === 'canvas' ? shown : hidden;
    const index = list.indexOf(part);
    const neighbour = list[index + step];
    if (!neighbour) return;
    onchange({ order: reorder(part, neighbour, step > 0) });
    requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-builder-part="${part}"]`)?.focus());
  }

  const markOf = (part: OverlayItem) =>
    target?.zone === 'canvas' && target.ref === part && drag?.part !== part ? `${target.after ? 'after' : 'before'}-${target.axis}` : '';
  const partLabel = (part: OverlayItem) => t(`settings.overlay.part.${part}`);
  const chipClass = (part: OverlayItem, from: 'canvas' | 'tray') => from === 'tray' ? 'hidden-part' : isSwitch(part) ? 'switch' : part === 'volumes' ? 'volumes' : 'wide';
</script>

{#snippet face(part: OverlayItem)}
  {@const Icon = ICONS[part]}
  {#if part === 'volumes'}
    <span class="hud-label volumes-label">{partLabel(part)}</span>
    <span class="strips" aria-hidden="true">
      {#each strips as name, index (index)}
        <span class="strip" class:master={index === 0}><i style:--fill="{70 - index * 12}%"></i><small>{name}</small></span>
      {/each}
    </span>
  {:else if isSwitch(part)}
    <Icon size={14} strokeWidth={1.5} /><span class="label">{partLabel(part)}</span>
  {:else}
    <span class="wide-face">
      <span class="hud-label">{partLabel(part)}</span>
      {#if part === 'mic_level'}
        <span class="meter" aria-hidden="true"><i></i></span>
      {:else}
        <span class="nav" aria-hidden="true"><ChevronLeft size={11} strokeWidth={1.5} /><Icon size={12} strokeWidth={1.5} /><ChevronRight size={11} strokeWidth={1.5} /></span>
      {/if}
    </span>
  {/if}
{/snippet}

{#snippet trayFace(part: OverlayItem)}
  {@const Icon = ICONS[part]}
  <GripVertical size={12} strokeWidth={1.5} /><Icon size={14} strokeWidth={1.5} /><span class="label">{partLabel(part)}</span>
{/snippet}

{#snippet chip(part: OverlayItem, from: 'canvas' | 'tray')}
  <button
    type="button"
    class="part {chipClass(part, from)} {from === 'canvas' ? markOf(part) : ''}"
    class:dragging={drag?.moved && drag.part === part}
    data-control={from === 'canvas' && part !== 'volumes' ? part : undefined}
    data-builder-part={part}
    aria-label={from === 'canvas' ? t('settings.overlay.builder.shownPart', { name: partLabel(part) }) : t('settings.overlay.builder.hiddenPart', { name: partLabel(part) })}
    title={t(`settings.overlay.partHint.${part}`)}
    onpointerdown={(event) => onpointerdown(event, part, from)}
    {onpointermove}
    {onpointerup}
    onpointercancel={oncancel}
    onlostpointercapture={oncancel}
    onkeydown={(event) => onkeydown(event, part, from)}
  >
    {#if from === 'tray'}
      {@render trayFace(part)}
    {:else}
      {@render face(part)}
    {/if}
  </button>
{/snippet}

<svelte:window onblur={oncancel} onkeydown={(event) => { if (event.key === 'Escape') oncancel(); }} onpointerout={(event) => { if (event.relatedTarget === null) oncancel(); }} />

<div class="builder" class:busy={drag?.moved}>
  <div class="stage">
    <span class="hud-label caption">{t('settings.overlay.builder.preview')}</span>
    <div
      class="canvas"
      class:right={!volumesLeft}
      class:drop={target?.zone === 'canvas'}
      class:drop-start={target?.zone === 'canvas' && drag?.part === 'volumes' && target.ref === 'start'}
      class:drop-end={target?.zone === 'canvas' && drag?.part === 'volumes' && target.ref === 'end'}
      class:no-volumes={!shown.includes('volumes')}
      bind:this={canvas}
      role="list"
      aria-label={t('settings.overlay.builder.preview')}
    >
      {#if shown.includes('volumes')}
        <div class="volumes-slot" role="listitem">{@render chip('volumes', 'canvas')}</div>
      {/if}
      <div class="column" class:drop-empty={target?.zone === 'canvas' && drag?.part !== 'volumes' && !shown.some((part) => part !== 'volumes' && part !== drag?.part)}>
        {#each rows as row, index (row.kind === 'wide' ? row.part : `switches-${index}`)}
          {#if row.kind === 'switches'}
            <div class="switches" role="listitem">
              {#each row.parts as part (part)}{@render chip(part, 'canvas')}{/each}
            </div>
          {:else}
            <div class="row" class:last={index === rows.length - 1} role="listitem">{@render chip(row.part, 'canvas')}</div>
          {/if}
        {:else}
          <p class="empty">{t('settings.overlay.builder.emptyColumn')}</p>
        {/each}
      </div>
    </div>
  </div>

  <div class="tray" class:drop={target?.zone === 'tray' && drag?.from === 'canvas'} bind:this={tray} role="list" aria-label={t('settings.overlay.builder.hidden')}>
    <span class="hud-label caption">{t('settings.overlay.builder.hidden')}</span>
    <div class="tray-parts">
      {#each hidden as part (part)}
        <span role="listitem">{@render chip(part, 'tray')}</span>
      {:else}
        <p class="empty">{t('settings.overlay.builder.emptyTray')}</p>
      {/each}
    </div>
  </div>

  <div class="foot">
    <p class="hint">{t('settings.overlay.builder.hint')}</p>
    <button class="btn small" disabled={isDefault} onclick={() => onchange({ order: [...OVERLAY_PARTS] })}>
      <RotateCcw size={12} strokeWidth={1.5} />{t('settings.overlay.builder.resetOrder')}
    </button>
  </div>
</div>

{#if drag?.moved}
  <div
    class="part ghost {chipClass(drag.part, drag.from)}"
    aria-hidden="true"
    style:left="{toCss(drag.x - drag.dx)}px"
    style:top="{toCss(drag.y - drag.dy)}px"
    style:width="{toCss(drag.width)}px"
    style:height="{toCss(drag.height)}px"
  >
    {#if drag.from === 'tray'}
      {@render trayFace(drag.part)}
    {:else}
      {@render face(drag.part)}
    {/if}
  </div>
{/if}

<style>
  .builder {
    display: grid;
    gap: 12px;
  }

  .busy,
  .busy :global(*) {
    cursor: grabbing;
    user-select: none;
    -webkit-user-select: none;
  }

  :global(.overlay-builder-dragging),
  :global(.overlay-builder-dragging *) {
    cursor: grabbing !important;
    user-select: none !important;
    -webkit-user-select: none !important;
  }

  .caption {
    display: block;
    margin-bottom: 8px;
    font-size: 9px;
    color: var(--faint);
  }

  .stage {
    padding: 12px;
    border: 1px solid var(--line);
    background: var(--bg);
    background-image: linear-gradient(var(--scanline) 1px, transparent 1px), linear-gradient(90deg, var(--scanline) 1px, transparent 1px);
    background-size: 12px 12px;
  }

  /* The overlay in small: volumes block and controls column, like HotkeyOverlay. */
  .canvas {
    position: relative;
    display: flex;
    gap: 12px;
    min-height: 196px;
    padding: 14px;
    border: 1px solid var(--line-strong);
    background: var(--bg-raised);
  }

  .canvas.drop {
    border-color: var(--accent);
  }

  .column.drop-empty {
    outline: 1px dashed var(--accent);
    outline-offset: 2px;
  }

  .canvas.right {
    flex-direction: row-reverse;
  }

  .canvas::before,
  .canvas::after {
    content: '';
    position: absolute;
    top: 8px;
    bottom: 8px;
    width: 2px;
    background: var(--accent);
    opacity: 0;
    transition: opacity var(--dur) var(--ease);
  }

  .canvas::before { left: 4px; }
  .canvas::after { right: 4px; }
  .canvas.drop-start::before,
  .canvas.drop-end::after {
    opacity: 1;
  }

  .volumes-slot {
    flex: 1;
    min-width: 0;
    display: flex;
  }

  .column {
    flex: none;
    width: 230px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-left: 12px;
    border-left: 1px solid var(--line);
  }

  .canvas.right .column {
    padding: 0 12px 0 0;
    border-left: 0;
    border-right: 1px solid var(--line);
  }

  .canvas.no-volumes .column {
    flex: 1;
    max-width: 260px;
    margin: 0 auto;
    padding: 0;
    border: 0;
  }

  .switches {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
  }

  .switches > :global(:last-child:nth-child(odd)) {
    grid-column: span 2;
  }

  .row.last {
    margin-top: auto;
  }

  .part {
    position: relative;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--muted);
    font: inherit;
    font-size: 11px;
    text-align: left;
    cursor: grab;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
    transition:
      border-color var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  .part:hover {
    border-color: var(--line-strong);
    color: var(--ink);
  }

  .part:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 2px;
    z-index: 1;
  }

  .part.dragging {
    opacity: 0.35;
    border-style: dashed;
  }

  .part :global(svg) {
    flex: none;
  }

  .label {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .switch {
    flex-direction: column;
    justify-content: center;
    gap: 4px;
    width: 100%;
    height: 46px;
    padding: 4px;
    text-align: center;
  }

  .switch .label {
    max-width: 100%;
    font-size: 10px;
  }

  .wide {
    width: 100%;
    padding: 6px 8px;
  }

  .wide-face {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 5px;
  }

  .wide-face .hud-label {
    font-size: 9px;
  }

  .nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--faint);
  }

  .meter {
    height: 4px;
    background: var(--bg);
  }

  .meter i {
    display: block;
    width: 62%;
    height: 100%;
    background: var(--app-lime-ink);
  }

  .volumes {
    flex: 1;
    flex-direction: column;
    align-items: stretch;
    padding: 8px;
  }

  .volumes-label {
    font-size: 9px;
    color: var(--faint);
  }

  .strips {
    flex: 1;
    display: flex;
    gap: 10px;
    justify-content: space-around;
  }

  .strip {
    flex: 1;
    max-width: 56px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .strip i {
    position: relative;
    flex: 1;
    width: 6px;
    min-height: 90px;
    background: linear-gradient(to top, var(--accent) var(--fill), var(--bg) var(--fill));
  }

  .strip.master i {
    background: linear-gradient(to top, var(--app-teal) var(--fill), var(--bg) var(--fill));
  }

  .strip small {
    max-width: 100%;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 9px;
    color: var(--faint);
  }

  /* Where a dragged part lands: a bar on the side of its neighbour. */
  .part::after {
    content: '';
    position: absolute;
    background: var(--accent);
    opacity: 0;
    pointer-events: none;
  }

  .part.before-x::after,
  .part.after-x::after {
    top: -2px;
    bottom: -2px;
    width: 2px;
    opacity: 1;
  }

  .part.before-x::after { left: -5px; }
  .part.after-x::after { right: -5px; }

  .part.before-y::after,
  .part.after-y::after {
    left: -2px;
    right: -2px;
    height: 2px;
    opacity: 1;
  }

  .part.before-y::after { top: -5px; }
  .part.after-y::after { bottom: -5px; }

  .tray {
    padding: 12px;
    border: 1px dashed var(--line);
    transition:
      border-color var(--dur) var(--ease),
      background var(--dur) var(--ease);
  }

  .tray.drop {
    border-color: var(--accent);
    background: var(--bg-sunken);
  }

  .tray-parts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    min-height: 30px;
  }

  .hidden-part {
    flex: none;
    height: 30px;
    max-width: 100%;
    white-space: nowrap;
    padding: 6px 10px 6px 6px;
    color: var(--faint);
  }

  .tray-parts > span {
    display: flex;
    max-width: 100%;
  }

  .empty {
    align-self: center;
    margin: auto;
    font-size: 12px;
    color: var(--faint);
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .hint {
    font-size: 12px;
    color: var(--muted);
  }

  .foot .btn {
    flex: none;
    gap: 6px;
  }

  .ghost {
    position: fixed;
    z-index: 100;
    margin: 0;
    border-color: var(--accent);
    color: var(--ink-strong);
    pointer-events: none;
    transition: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .part,
    .tray,
    .canvas::before,
    .canvas::after {
      transition: none;
    }
  }
</style>
