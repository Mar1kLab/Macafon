<script lang="ts">
  import { onMount } from 'svelte';
  import { RefreshCw, Upload } from '@lucide/svelte';
  import Sidebar, { type Screen } from './lib/components/Sidebar.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import Ticks from './lib/components/Ticks.svelte';
  import Microphone from './screens/Microphone.svelte';
  import Settings from './screens/Settings.svelte';
  import Mixer from './screens/Mixer.svelte';
  import Soundpad from './screens/Soundpad.svelte';
  import Dictation from './screens/Dictation.svelte';
  import Onboarding from './lib/onboarding/Onboarding.svelte';
  import { daemon } from './lib/state';
  import { t } from './lib/i18n';

  const initial = new URLSearchParams(location.search).get('screen');
  let screen = $state<Screen>(initial === 'settings' || initial === 'mixer' || initial === 'soundpad' || initial === 'dictation' ? initial : 'microphone');

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
    screen = 'soundpad';
    const added = await daemon.uploadSounds(files, null);
    if (added > 0) daemon.toast(t('soundpad.added', { count: added }));
  }

  // UI scale 90–150% with Ctrl +/− and Ctrl 0 (PRD §9.8). The layout never gets smaller than
  // the minimum window (960×640 CSS pixels): a small window caps the scale it applies.
  const ZOOM_STEPS = [0.9, 1, 1.1, 1.25, 1.5];
  const MIN_LAYOUT = { width: 960, height: 640 };
  let zoom = $state(1);
  let viewport = $state({ width: window.innerWidth, height: window.innerHeight });
  const applied = $derived(
    Math.max(ZOOM_STEPS[0]!, Math.min(zoom, viewport.width / MIN_LAYOUT.width, viewport.height / MIN_LAYOUT.height)),
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

{#if daemon.onboarding && daemon.mic && daemon.status.state === 'connected'}
  <Onboarding
    onfinish={(fineTune) => {
      daemon.finishOnboarding();
      if (fineTune) screen = 'microphone';
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
    <Sidebar {screen} onnavigate={(next) => (screen = next)} />
    <main>
      {#if daemon.status.state === 'unavailable'}
        <div class="unavailable" role="alert">
          <div class="panel">
            <Ticks />
            <span class="hud-label">{t('daemon.unavailable')}</span>
            <h1>{t('daemon.unavailableTitle')} <em>{t('daemon.unavailableAccent')}</em>.</h1>
            <p>{t('daemon.unavailableBody')}</p>
            <p class="reason">{daemon.status.reason}</p>
            <button class="btn invert" onclick={() => daemon.retry()}>
              <RefreshCw size={13} strokeWidth={1.5} />{t('daemon.retry')}
            </button>
          </div>
        </div>
      {:else if screen === 'settings'}
        <Settings onnavigate={(next) => (screen = next)} />
      {:else if screen === 'mixer'}
        <Mixer />
      {:else if screen === 'soundpad' && daemon.soundpadAvailable}
        <Soundpad />
      {:else if screen === 'dictation' && daemon.dictationAvailable}
        <Dictation />
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
<Toasts />

<style>
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
    gap: 12px;
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

  .panel {
    position: relative;
    display: grid;
    gap: 12px;
    justify-items: start;
    max-width: 480px;
    padding: 32px;
    border: 1px solid var(--line);
    background: var(--bg-raised);
  }

  h1 {
    font-family: var(--font-display);
    font-size: 32px;
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
    line-height: 1.1;
  }

  em {
    font-family: var(--font-serif);
    font-weight: 400;
    letter-spacing: 0;
  }

  p {
    color: var(--muted);
  }

  .reason {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--danger);
  }
</style>
