<script lang="ts">
  // App chip (design system §6, APP-003): icon, readable name and a pulsing dot while the app
  // plays. Drag it to another channel (APP-004), or focus it and press 1…8 / open its menu
  // (APP-008). Remembered apps that are not running are dimmed (APP-007). A click opens the
  // app's own volume inside the channel (CH-008).
  import { prefersReducedMotion } from 'svelte/motion';
  import { tick } from 'svelte';
  import { uiZoom } from '../components/zoom';
  import { Lock } from '@lucide/svelte';
  import RulerSlider from '../components/RulerSlider.svelte';
  import Menu, { type MenuAnchor, type MenuEntry } from '../components/Menu.svelte';
  import type { AppState, ChannelState } from '../ipc/protocol';
  import { t } from '../i18n';
  import { daemon } from '../state';
  import { tip } from '../components/tooltip.svelte';

  interface Props {
    app: AppState;
    channels: ChannelState[];
    color: string;
    onmove: (channel: string) => void;
    onvolume: (volume_pct: number) => void;
    onmanaged: (managed: boolean) => void;
    onforget: () => void;
    ondragstart: (key: string) => void;
    ondragend: () => void;
  }

  let { app, channels, color, onmove, onvolume, onmanaged, onforget, ondragstart, ondragend }: Props = $props();
  let menu = $state<MenuAnchor | null>(null);
  let volume = $state<{ x: number; y: number } | null>(null);
  let popover: HTMLDivElement | undefined = $state();
  const fixed = $derived(!app.movable || !app.managed);
  const initial = $derived(app.name.trim().charAt(0).toUpperCase() || '?');
  const icon = $derived(daemon.appIcon(app.icon));

  /**
   * Keep a fixed popover inside the window: shifted left, or flipped above `y - flip`. `at` and
   * `flip` are screen pixels; the result is in CSS pixels for `left`/`top` under the UI zoom.
   */
  function inside(element: HTMLElement | undefined, at: { x: number; y: number }, flip: number) {
    const zoom = uiZoom();
    if (!element) return { x: at.x / zoom, y: at.y / zoom };
    const margin = 8;
    const { width, height } = element.getBoundingClientRect();
    const x = Math.max(margin, Math.min(at.x, window.innerWidth - width - margin));
    const fits = at.y + height <= window.innerHeight - margin;
    const y = fits ? at.y : Math.max(margin, Math.min(at.y - flip - height, window.innerHeight - height - margin));
    return { x: x / zoom, y: y / zoom };
  }

  /** "Move to" the channels by number, (un)manage, forget a remembered app (APP-008). */
  const entries = $derived<MenuEntry[]>([
    { kind: 'heading', label: t('mixer.moveTo') },
    ...channels.map((channel, index): MenuEntry => ({
      kind: 'item',
      label: channel.name,
      swatch: `var(--app-${channel.color})`,
      kbd: String(index + 1),
      disabled: channel.id === app.channel || !app.movable || !channel.enabled,
      onselect: () => onmove(channel.id),
    })),
    { kind: 'separator' },
    { kind: 'check', label: t('mixer.dontManage'), checked: !app.managed, onselect: () => onmanaged(!app.managed) },
    ...(app.streams === 0 ? [{ kind: 'item', label: t('mixer.forget'), onselect: onforget } as MenuEntry] : []),
  ]);

  function closeMenu() {
    menu = null;
    // Escape or a chosen item: keyboard focus goes back to the chip, not to the page.
    queueMicrotask(() => {
      if (document.activeElement === document.body) chipButton?.focus();
    });
  }

  function onkeydown(event: KeyboardEvent) {
    const index = Number(event.key) - 1;
    if (Number.isInteger(index) && index >= 0 && index < channels.length) {
      event.preventDefault();
      const target = channels[index]!;
      if (target.id !== app.channel && target.enabled) onmove(target.id);
    } else if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
      event.preventDefault();
      menu = event.currentTarget as HTMLElement;
    }
  }

  /** The lifted chip follows the cursor tilted by 2° (design system §5). */
  function liftedImage(chip: HTMLElement, transfer: DataTransfer, event: DragEvent) {
    if (prefersReducedMotion.current) return;
    const rect = chip.getBoundingClientRect();
    const ghost = chip.cloneNode(true) as HTMLElement;
    ghost.style.cssText = `position:fixed;top:-200px;left:0;width:${rect.width}px;transform:rotate(2deg);`;
    document.body.appendChild(ghost);
    transfer.setDragImage(ghost, event.clientX - rect.left, event.clientY - rect.top);
    setTimeout(() => ghost.remove());
  }

  async function openVolume(chip: HTMLElement) {
    const rect = chip.getBoundingClientRect();
    const at = { x: rect.left, y: rect.bottom + 4 };
    volume = inside(undefined, at, 0);
    await tick();
    volume = inside(popover, at, rect.height + 8);
    popover?.querySelector<HTMLElement>('[role="slider"]')?.focus();
  }

  function closeVolume(refocus: boolean) {
    volume = null;
    if (refocus) chipButton?.focus();
  }

  let chipButton: HTMLButtonElement | undefined = $state();

</script>

<svelte:window
  onpointerdown={(event) => {
    if (volume && !popover?.contains(event.target as Node) && !chipButton?.contains(event.target as Node)) volume = null;
  }}
/>

<button
  bind:this={chipButton}
  class="chip"
  class:inactive={app.streams === 0}
  class:fixed
  draggable={app.movable && app.managed}
  style:--c="var(--app-{color})"
  style:--c-ink="var(--app-{color}-ink)"
  {@attach tip(!app.movable ? t('mixer.cannotMove') : !app.managed ? t('mixer.unmanagedHint') : t('mixer.chipHint'))}
  aria-label="{app.name}{app.active ? ` · ${t('mixer.playing')}` : ''}"
  {onkeydown}
  onclick={(event) => (volume ? closeVolume(false) : void openVolume(event.currentTarget as HTMLElement))}
  oncontextmenu={(event) => {
    event.preventDefault();
    menu = { x: event.clientX, y: event.clientY };
  }}
  ondragstart={(event) => {
    const transfer = event.dataTransfer;
    if (!transfer) return;
    transfer.setData('application/x-macafon-app', app.key);
    transfer.effectAllowed = 'move';
    liftedImage(event.currentTarget as HTMLElement, transfer, event);
    ondragstart(app.key);
  }}
  ondragend={ondragend}
>
  {#if icon}
    <img class="icon image" src={icon} alt="" draggable="false" />
  {:else}
    <span class="icon" aria-hidden="true">{initial}</span>
  {/if}
  <span class="name">{app.name}</span>
  {#if app.volume_pct < 100}<span class="vol">{Math.round(app.volume_pct)}%</span>{/if}
  {#if fixed}<Lock size={11} strokeWidth={1.5} />{/if}
  {#if app.active}<span class="dot" aria-hidden="true"></span>{/if}
</button>

{#if volume}
  <div
    class="volume"
    role="dialog"
    aria-label={t('mixer.appVolume', { name: app.name })}
    tabindex="-1"
    bind:this={popover}
    style:left="{volume.x}px"
    style:top="{volume.y}px"
    onkeydown={(event) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeVolume(true);
      }
    }}
  >
    <RulerSlider
      value={app.volume_pct}
      min={0}
      max={100}
      step={1}
      defaultValue={100}
      label={t('mixer.appVolumeShort')}
      format={(v) => `${Math.round(v)}%`}
      {color}
      onchange={onvolume}
    />
  </div>
{/if}

<Menu anchor={menu} label={t('mixer.moveTo')} {entries} onclose={closeMenu} />

<style>
  .chip {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--line);
    background: var(--bg-raised);
    color: var(--ink);
    font-size: var(--text-md);
    text-align: left;
    cursor: grab;
    transition:
      transform var(--dur) var(--ease),
      border-color var(--dur) var(--ease),
      filter var(--dur) var(--ease);
  }

  .chip:hover {
    transform: translateY(-4px);
    filter: drop-shadow(0 6px 14px rgba(0, 0, 0, 0.28));
    border-color: var(--line-strong);
  }

  .chip:active {
    cursor: grabbing;
  }

  .chip.fixed {
    cursor: default;
  }

  .chip.inactive {
    opacity: 0.45;
  }

  .icon {
    flex: none;
    width: 20px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--c) 30%, transparent);
    color: var(--c-ink);
    font-family: var(--font-display);
    font-size: var(--text-sm);
    font-weight: 700;
  }

  .icon.image {
    object-fit: contain;
    background: none;
  }

  .vol {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--muted);
  }

  .volume {
    position: fixed;
    z-index: 40;
    width: 260px;
    padding: 10px 12px;
    background: var(--bg-raised);
    border: 1px solid var(--line-strong);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    background: var(--c-ink);
    animation: pulse var(--dur-loop) ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }








  @media (prefers-reduced-motion: reduce) {
    .chip:hover {
      transform: none;
    }

    .dot {
      animation: none;
    }
  }
</style>
