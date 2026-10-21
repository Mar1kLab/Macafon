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
  import { daemon } from '../state';

  type Mode = 'tray' | 'overlay';

  const tauri = isTauri();
  const initial = new URLSearchParams(location.search).get('mode');
  let mode = $state<Mode>(initial === 'overlay' ? 'overlay' : 'tray');
  let root: HTMLDivElement | undefined = $state();
  /** Bumped every time the daemon asks to show a panel: the overlay starts collapsed again. */
  let opened = $state(0);

  // The Sketch look draws no system frame: the page goes transparent around its lines and the
  // window drops its shadow and corners on the next fit.
  $effect(() => {
    document.documentElement.classList.toggle('frameless', daemon.sketch);
    void fit();
  });

  function measure() {
    const rect = root?.getBoundingClientRect();
    return { width: Math.ceil(rect?.width ?? 0), height: Math.ceil(rect?.height ?? 0) };
  }

  let fitting = false;
  let refit = false;

  /** Serialize layout changes so a native resize cannot queue stale measurements. The
   * reveal transition reserves its final size throughout the animation. */
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
            width,
            height,
            anchorLeft: main ? main.left - origin.left : null,
            anchorRight: main ? main.right - origin.left : null,
            frameless: daemon.sketch,
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
    const observer = new ResizeObserver(() => void fit());
    if (root) observer.observe(root);
    window.addEventListener('keydown', onkey);
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
  /* Sketch: no system frame; the desktop shows around the panel's lines (§9.9.2). */
  :global(html.frameless),
  :global(html.frameless body) {
    background: transparent;
    background-image: none;
  }

  .fit {
    width: max-content;
  }

  .fit.tray {
    width: 360px;
  }
</style>
