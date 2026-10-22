<script lang="ts">
  import { onMount } from 'svelte';
  import { Upload } from '@lucide/svelte';
  import RefreshCw from './lib/icons/animated/RefreshCw.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import TitleBar from './lib/components/TitleBar.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import TooltipLayer from './lib/components/TooltipLayer.svelte';
  import Ticks from './lib/components/Ticks.svelte';
  import EmptyState from './lib/components/EmptyState.svelte';
  import Microphone from './screens/Microphone.svelte';
  import Onboarding from './lib/onboarding/Onboarding.svelte';
  import { daemon } from './lib/state';
  import { nav } from './lib/state/nav.svelte';
  import { t } from './lib/i18n';

  const screen = $derived(nav.screen);

  // The other screens load on first use and in idle time after the first paint: the window
  // opens with only the Microphone screen to parse, and switching later finds them ready.
  const loaders = {
    settings: () => import('./screens/Settings.svelte'),
    mixer: () => import('./screens/Mixer.svelte'),
    soundpad: () => import('./screens/Soundpad.svelte'),
    dictation: () => import('./screens/Dictation.svelte'),
    displays: () => import('./screens/Displays.svelte'),
  };
  type Lazy = keyof typeof loaders;
  type Loaded = { [K in Lazy]?: Awaited<ReturnType<(typeof loaders)[K]>>['default'] };
  let screens = $state.raw<Loaded>({});

  function load(name: Lazy) {
    if (screens[name]) return;
    void loaders[name]().then((module) => (screens = { ...screens, [name]: module.default }));
  }

  $effect(() => {
    if (screen in loaders) load(screen as Lazy);
  });

  onMount(() => {
    const prefetch = () => (Object.keys(loaders) as Lazy[]).forEach(load);
    if ('requestIdleCallback' in window) {
      const id = requestIdleCallback(prefetch, { timeout: 3000 });
      return () => cancelIdleCallback(id);
    }
    const timer = setTimeout(prefetch, 1000);
    return () => clearTimeout(timer);
  });

  // SND-001: audio files dropped anywhere in the window go to the Soundpad; its own screen
  // takes drops itself, into the category on screen.
  let fileDrag = $state(false);
  const hasFiles = (event: DragEvent) => daemon.soundpadAvailable && (event.dataTransfer?.types.includes('Files') ?? false);

  function ondragover(event: DragEvent) {
    // Without this the WebView would open a dropped file in place of the app.
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = hasFiles(event) ? 'copy' : 'none';
    fileDrag = hasFiles(event);
  }

  async function ondrop(event: DragEvent) {
    event.preventDefault();
    fileDrag = false;
    if (!hasFiles(event)) return;
    const files = [...(event.dataTransfer?.files ?? [])];
    nav.go('soundpad');
    const added = await daemon.uploadSounds(files, null);
    if (added > 0) daemon.toast(t('soundpad.added', { count: added }));
  }

  // UI scale 90–150% with Ctrl +/− and Ctrl 0 (PRD §9.8). The layout never gets smaller than
  // the minimum window (960×640 CSS pixels): a small window caps the scale it applies.
  const ZOOM_STEPS = [0.9, 1, 1.1, 1.25, 1.5];
  const MIN_LAYOUT = { width: 960, height: 640 };
  // The title bar's 32 px come off the height the layout gets.
  const TITLE_BAR = 32;
  let zoom = $state(1);
  let viewport = $state({ width: window.innerWidth, height: window.innerHeight });
  const applied = $derived(
    Math.max(ZOOM_STEPS[0]!, Math.min(zoom, viewport.width / MIN_LAYOUT.width, (viewport.height - TITLE_BAR) / MIN_LAYOUT.height)),
  );

  onMount(() => {
    const onkey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      const index = ZOOM_STEPS.indexOf(zoom);
      if (event.key === '=' || event.key === '+') zoom = ZOOM_STEPS[Math.min(index + 1, ZOOM_STEPS.length - 1)]!;
      else if (event.key === '-') zoom = ZOOM_STEPS[Math.max(index - 1, 0)]!;
      else if (event.key === '0') zoom = 1;
      else return;
      event.preventDefault();
    };
    const onresize = () => (viewport = { width: window.innerWidth, height: window.innerHeight });
    window.addEventListener('keydown', onkey);
    window.addEventListener('resize', onresize);
    return () => {
      window.removeEventListener('keydown', onkey);
      window.removeEventListener('resize', onresize);
    };
  });

  $effect(() => {
    document.documentElement.style.setProperty('zoom', String(applied));
    // Viewport units scale with `zoom`; layouts divide by it to fit the window.
    document.documentElement.style.setProperty('--zoom', String(applied));
  });
</script>

<div class="window">
<TitleBar title={t(`nav.${screen}`)} />
{#if daemon.onboarding && daemon.mic && daemon.status.state === 'connected'}
  <Onboarding
    onfinish={(fineTune) => {
      daemon.finishOnboarding();
      if (fineTune) nav.go('microphone');
    }}
  />
{:else}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="app"
    {ondragover}
    ondropcapture={() => (fileDrag = false)}
    ondragleave={(event) => {
      if (!event.relatedTarget) fileDrag = false;
    }}
    {ondrop}
  >
    <Sidebar {screen} onnavigate={(next) => nav.go(next)} />
    <main>
      {#if daemon.status.state === 'unavailable'}
        <div class="unavailable">
          <EmptyState role="alert" tone="danger" label={t('daemon.unavailable')} title={t('daemon.unavailableTitle')} accent={t('daemon.unavailableAccent')} width="480px">
            <p>{t('daemon.unavailableBody')}</p>
            <p class="reason">{daemon.status.reason}</p>
            {#snippet actions()}
              <button class="btn invert" onclick={() => daemon.retry()}>
                <RefreshCw size={13} strokeWidth={1.5} />{t('daemon.retry')}
              </button>
            {/snippet}
          </EmptyState>
        </div>
      {:else if screen === 'settings'}
        {#if screens.settings}<screens.settings onnavigate={(next) => nav.go(next)} />{/if}
      {:else if screen === 'mixer'}
        {#if screens.mixer}<screens.mixer />{/if}
      {:else if screen === 'soundpad' && daemon.soundpadAvailable}
        {#if screens.soundpad}<screens.soundpad />{/if}
      {:else if screen === 'dictation' && daemon.dictationAvailable}
        {#if screens.dictation}<screens.dictation />{/if}
      {:else if screen === 'displays' && daemon.displaysAvailable}
        {#if screens.displays}<screens.displays />{/if}
      {:else}
        <Microphone />
      {/if}
    </main>
    {#if fileDrag && screen !== 'soundpad'}
      <div class="dropzone" aria-hidden="true">
        <Ticks color="var(--accent)" />
        <Upload size={22} strokeWidth={1.5} />
        <span class="hud-label">{t('soundpad.dropApp')}</span>
      </div>
    {/if}
  </div>
{/if}
</div>
<Toasts />
<TooltipLayer />

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .window > :global(:not(.titlebar)) {
    flex: 1;
    min-height: 0;
    height: auto;
  }

  .app {
    position: relative;
    display: flex;
    height: 100%;
    min-width: 0;
    min-height: 0;
    background-color: var(--bg);
    background-image: linear-gradient(var(--scanline) 1px, transparent 1px), linear-gradient(90deg, var(--scanline) 1px, transparent 1px);
    background-size: 24px 24px;
  }

  .dropzone {
    position: absolute;
    inset: 16px 16px 16px calc(var(--sidebar-width) + 16px);
    z-index: 50;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    border: 1px dashed var(--accent);
    background: color-mix(in srgb, var(--bg) 90%, transparent);
    color: var(--accent-ink);
    pointer-events: none;
  }

  .dropzone .hud-label {
    color: var(--accent-ink);
  }

  main {
    flex: 1;
    min-width: 0;
    height: 100%;
  }

  .unavailable {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .reason {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--danger);
  }
</style>
