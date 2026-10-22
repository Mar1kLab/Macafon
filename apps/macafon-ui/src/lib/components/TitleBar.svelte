<script lang="ts">
  // The main window's own title bar (the window has no system frame): the app's look instead of
  // the Windows caption, with a drag region and the minimize / maximize / close buttons. Double
  // click on the bar maximizes, like the system one. In a browser (mock daemon) the buttons do
  // nothing.
  import { onMount } from 'svelte';
  import { Copy, Minus, Square, X } from '@lucide/svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Logo from './Logo.svelte';
  import { isTauri } from '../ipc/tauri';
  import { t } from '../i18n';

  let { title }: { title: string } = $props();
  const tauri = isTauri();
  const appWindow = tauri ? getCurrentWindow() : null;
  let maximized = $state(false);
  let focused = $state(true);

  onMount(() => {
    if (!appWindow) return;
    const sync = async () => {
      maximized = await appWindow.isMaximized().catch(() => false);
    };
    void sync();
    const unlisten = [appWindow.onResized(() => void sync()), appWindow.onFocusChanged(({ payload }) => (focused = payload))];
    return () => unlisten.forEach((pending) => void pending.then((stop) => stop()));
  });
</script>

<header class="titlebar" class:blurred={!focused} data-tauri-drag-region>
  <span class="name" data-tauri-drag-region>
    <span class="logo" aria-hidden="true"><Logo label="" /></span>
    <b data-tauri-drag-region>{t('app.name')}</b>
    <span class="hud-label" data-tauri-drag-region>{title}</span>
  </span>
  <span class="controls">
    <button aria-label={t('window.minimize')} title={t('window.minimize')} onclick={() => appWindow?.minimize()}><Minus size={14} strokeWidth={1.5} /></button>
    <button aria-label={maximized ? t('window.restore') : t('window.maximize')} title={maximized ? t('window.restore') : t('window.maximize')} onclick={() => appWindow?.toggleMaximize()}>
      {#if maximized}<Copy size={12} strokeWidth={1.5} />{:else}<Square size={11} strokeWidth={1.5} />{/if}
    </button>
    <button class="close" aria-label={t('window.close')} title={t('window.close')} onclick={() => appWindow?.close()}><X size={15} strokeWidth={1.5} /></button>
  </span>
</header>

<style>
  .titlebar {
    flex: none;
    display: flex;
    align-items: stretch;
    height: 32px;
    border-bottom: 1px solid var(--line);
    background: var(--bg-raised);
    user-select: none;
    -webkit-user-select: none;
  }

  .name {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding-left: 12px;
    transition: opacity var(--dur) var(--ease);
  }

  .logo {
    display: inline-flex;
    width: 14px;
    height: 16px;
    pointer-events: none;
  }

  .logo :global(svg) {
    width: 100%;
    height: 100%;
  }

  b {
    font-family: var(--font-display);
    font-size: var(--text-md);
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--ink-strong);
  }

  .hud-label {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    padding-left: 10px;
    border-left: 1px solid var(--line);
    font-size: var(--text-2xs);
    color: var(--faint);
  }

  .blurred .name {
    opacity: 0.6;
  }

  .controls {
    display: flex;
  }

  button {
    width: 46px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-left: 1px solid transparent;
    background: transparent;
    color: var(--muted);
    cursor: default;
    transition:
      background var(--dur) var(--ease),
      color var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  button:hover {
    background: var(--bg-sunken);
    border-left-color: var(--line);
    color: var(--ink-strong);
  }

  button:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: -2px;
  }

  .close:hover {
    background: var(--danger);
    border-left-color: var(--danger);
    color: var(--ink-strong);
  }

  @media (prefers-reduced-motion: reduce) {
    .name,
    button {
      transition: none;
    }
  }
</style>
