<script lang="ts">
  // The panel window's page: the tray panel or the hotkey overlay. The window is created
  // hidden when the UI starts and kept, so this page stays loaded and connected; the daemon's
  // request arrives as `panel://show` with the layout to show. The page lays it out, measures
  // it and sends the size, and the window shows itself at that size (`quick_panel_fit`).
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import QuickPanel from './QuickPanel.svelte';
  import HotkeyOverlay from './HotkeyOverlay.svelte';
  import { isTauri } from '../ipc/tauri';

  type Mode = 'tray' | 'overlay';

  const tauri = isTauri();
  const initial = new URLSearchParams(location.search).get('mode');
  let mode = $state<Mode>(initial === 'overlay' ? 'overlay' : 'tray');
  let root: HTMLDivElement | undefined = $state();

  // Windows gives an undecorated window with a shadow a larger viewport than requested (its
  // hidden caption): what it gets beyond the request is taken off the next one.
  let extraWidth = 0;
  let extraHeight = 0;

  function measure() {
    const rect = root?.getBoundingClientRect();
    return { width: Math.ceil(rect?.width ?? 0), height: Math.ceil(rect?.height ?? 0) };
  }

  async function fit() {
    if (!root || !tauri) return;
    const { width, height } = measure();
    if (width > 0 && height > 0) {
      await invoke('quick_panel_fit', { width: width - extraWidth, height: height - extraHeight });
    }
  }

  async function show(next: Mode) {
    mode = next;
    await tick();
    await fit();
  }

  async function close() {
    if (tauri) await invoke('quick_panel_close');
  }

  onMount(() => {
    const onkey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        void close();
      }
    };
    const onresize = () => {
      const { width, height } = measure();
      const offWidth = window.innerWidth - width;
      const offHeight = window.innerHeight - height;
      if (Math.abs(offWidth) <= 1 && Math.abs(offHeight) <= 1) return;
      extraWidth += offWidth;
      extraHeight += offHeight;
      void fit();
    };
    const observer = new ResizeObserver(() => void fit());
    if (root) observer.observe(root);
    window.addEventListener('keydown', onkey);
    window.addEventListener('resize', onresize);
    let unlisten: (() => void) | undefined;
    if (tauri) {
      void (async () => {
        unlisten = await listen<Mode>('panel://show', (event) => void show(event.payload));
        // Opened for a panel before this page listened: the request is still pending.
        const pending = await invoke<Mode | null>('quick_panel_pending');
        if (pending) await show(pending);
      })();
    }
    return () => {
      observer.disconnect();
      window.removeEventListener('keydown', onkey);
      window.removeEventListener('resize', onresize);
      unlisten?.();
    };
  });
</script>

<div class="fit" class:tray={mode === 'tray'} bind:this={root}>
  {#if mode === 'overlay'}
    <HotkeyOverlay onclose={close} />
  {:else}
    <QuickPanel onclose={close} />
  {/if}
</div>

<style>
  .fit {
    width: max-content;
  }

  .fit.tray {
    width: 360px;
  }
</style>
