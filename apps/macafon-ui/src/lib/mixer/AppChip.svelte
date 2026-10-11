<script lang="ts">
  // App chip (design system §6, APP-003): icon, readable name and a pulsing dot while the app
  // plays. Drag it to another channel (APP-004), or focus it and press 1…8 / open its menu
  // (APP-008). Remembered apps that are not running are dimmed (APP-007).
  import { tick } from 'svelte';
  import { Lock } from '@lucide/svelte';
  import type { AppState, ChannelState } from '../ipc/protocol';
  import { t } from '../i18n';

  interface Props {
    app: AppState;
    channels: ChannelState[];
    color: string;
    onmove: (channel: string) => void;
    onmanaged: (managed: boolean) => void;
    onforget: () => void;
    ondragstart: (key: string) => void;
    ondragend: () => void;
  }

  let { app, channels, color, onmove, onmanaged, onforget, ondragstart, ondragend }: Props = $props();
  let menu = $state<{ x: number; y: number } | null>(null);
  let list: HTMLUListElement | undefined = $state();
  const fixed = $derived(!app.movable || !app.managed);
  const initial = $derived(app.name.trim().charAt(0).toUpperCase() || '?');

  async function openMenu(x: number, y: number) {
    menu = { x, y };
    await tick();
    list?.querySelector('button')?.focus();
  }

  function onkeydown(event: KeyboardEvent) {
    const index = Number(event.key) - 1;
    if (Number.isInteger(index) && index >= 0 && index < channels.length) {
      event.preventDefault();
      const target = channels[index]!;
      if (target.id !== app.channel) onmove(target.id);
    } else if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
      event.preventDefault();
      const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
      void openMenu(rect.left, rect.bottom);
    }
  }

  /** The lifted chip follows the cursor tilted by 2° (design system §5). */
  function liftedImage(chip: HTMLElement, transfer: DataTransfer, event: DragEvent) {
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const rect = chip.getBoundingClientRect();
    const ghost = chip.cloneNode(true) as HTMLElement;
    ghost.style.cssText = `position:fixed;top:-200px;left:0;width:${rect.width}px;transform:rotate(2deg);`;
    document.body.appendChild(ghost);
    transfer.setDragImage(ghost, event.clientX - rect.left, event.clientY - rect.top);
    setTimeout(() => ghost.remove());
  }

  function choose(action: () => void) {
    menu = null;
    action();
  }
</script>

<svelte:window onpointerdown={(event) => menu && !list?.contains(event.target as Node) && (menu = null)} />

<button
  class="chip"
  class:inactive={app.streams === 0}
  class:fixed
  draggable={app.movable && app.managed}
  style:--c="var(--app-{color})"
  style:--c-ink="var(--app-{color}-ink)"
  title={!app.movable ? t('mixer.cannotMove') : !app.managed ? t('mixer.unmanagedHint') : t('mixer.chipHint')}
  aria-label="{app.name}{app.active ? ` · ${t('mixer.playing')}` : ''}"
  {onkeydown}
  oncontextmenu={(event) => {
    event.preventDefault();
    void openMenu(event.clientX, event.clientY);
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
  <span class="icon" aria-hidden="true">{initial}</span>
  <span class="name">{app.name}</span>
  {#if fixed}<Lock size={11} strokeWidth={1.5} />{/if}
  {#if app.active}<span class="dot" aria-hidden="true"></span>{/if}
</button>

{#if menu}
  <ul class="menu" role="menu" bind:this={list} style:left="{menu.x}px" style:top="{menu.y}px">
    <li class="hud-label" role="presentation">{t('mixer.moveTo')}</li>
    {#each channels as channel, index (channel.id)}
      <li role="none">
        <button role="menuitem" disabled={channel.id === app.channel || !app.movable} onclick={() => choose(() => onmove(channel.id))}>
          <span class="swatch" style:background="var(--app-{channel.color})"></span>{channel.name}<kbd>{index + 1}</kbd>
        </button>
      </li>
    {/each}
    <li role="none">
      <button role="menuitemcheckbox" aria-checked={!app.managed} onclick={() => choose(() => onmanaged(!app.managed))}>
        {app.managed ? t('mixer.dontManage') : t('mixer.manage')}
      </button>
    </li>
    {#if app.streams === 0}
      <li role="none"><button role="menuitem" onclick={() => choose(onforget)}>{t('mixer.forget')}</button></li>
    {/if}
  </ul>
{/if}

<style>
  .chip {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--line);
    background: var(--bg-raised);
    color: var(--ink);
    font-size: 13px;
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
    font-size: 12px;
    font-weight: 700;
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
    border-radius: 50%;
    background: var(--c-ink);
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }

  .menu {
    position: fixed;
    z-index: 40;
    min-width: 180px;
    margin: 0;
    padding: 4px 0;
    list-style: none;
    background: var(--bg-raised);
    border: 1px solid var(--line-strong);
  }

  .menu .hud-label {
    padding: 6px 12px;
  }

  .menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 12px;
    text-align: left;
    color: var(--ink);
  }

  .menu button:hover:not(:disabled),
  .menu button:focus-visible {
    background: var(--bg-sunken);
  }

  .menu button:disabled {
    color: var(--faint);
  }

  .swatch {
    width: 8px;
    height: 8px;
  }

  kbd {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--faint);
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
