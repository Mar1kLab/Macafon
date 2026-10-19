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
  /** Bumped every time the daemon asks to show a panel: the overlay starts collapsed again. */
  let opened = $state(0);

  // Windows gives an undecorated window with a shadow a larger viewport than requested (its
  // hidden caption): what it gets beyond the request is taken off the next one.
  let extraWidth = 0;
  let extraHeight = 0;

  function measure() {
    const rect = root?.getBoundingClientRect();
    return { width: Math.ceil(rect?.width ?? 0), height: Math.ceil(rect?.height ?? 0) };
  }

  let fitting = false;
  let refit = false;

  /** One fit at a time: while a side panel opens or closes the page resizes every frame, and
   * the window follows with the latest size instead of a queue of stale ones. */
  async function fit() {
    if (!root || !tauri) return;
    if (fitting) {
      refit = true;
      return;
    }
    fitting = true;
    try {
      do {
        refit = false;
        const { width, height } = measure();
        // Anchored: the overlay's main part stays where it is and a side panel grows the window
        // away from it. Centered (or no main part): the whole window is centered.
        const main = root.querySelector('[data-overlay-main]:not([data-overlay-centered])')?.getBoundingClientRect();
        const origin = root.getBoundingClientRect();
        if (width > 0 && height > 0) {
          await invoke('quick_panel_fit', {
            width: width - extraWidth,
            height: height - extraHeight,
            anchorLeft: main ? main.left - origin.left : null,
            anchorRight: main ? main.right - origin.left : null,
          });
        }
      } while (refit);
    } finally {
      fitting = false;
    }
  }

  async function show(next: Mode) {
    mode = next;
    opened++;
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
    <HotkeyOverlay onclose={close} {opened} />
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
