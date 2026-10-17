<script lang="ts">
  import { onMount } from 'svelte';
  import { RefreshCw } from '@lucide/svelte';
  import Sidebar, { type Screen } from './lib/components/Sidebar.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import Ticks from './lib/components/Ticks.svelte';
  import Microphone from './screens/Microphone.svelte';
  import Settings from './screens/Settings.svelte';
  import Mixer from './screens/Mixer.svelte';
  import Onboarding from './lib/onboarding/Onboarding.svelte';
  import { daemon } from './lib/state';
  import { t } from './lib/i18n';

  const initial = new URLSearchParams(location.search).get('screen');
  let screen = $state<Screen>(initial === 'settings' || initial === 'mixer' ? initial : 'microphone');

  // UI scale 90–150% with Ctrl +/− and Ctrl 0 (PRD §9.8).
  const ZOOM_STEPS = [0.9, 1, 1.1, 1.25, 1.5];
  let zoom = $state(1);

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
    window.addEventListener('keydown', onkey);
    return () => window.removeEventListener('keydown', onkey);
  });

  $effect(() => {
    document.documentElement.style.setProperty('zoom', String(zoom));
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
  <div class="app">
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
      {:else}
        <Microphone />
      {/if}
    </main>
  </div>
{/if}
<Toasts />

<style>
  .app {
    display: flex;
    height: 100%;
    min-width: 0;
    min-height: 0;
    background-color: var(--bg);
    background-image: linear-gradient(var(--scanline) 1px, transparent 1px), linear-gradient(90deg, var(--scanline) 1px, transparent 1px);
    background-size: 24px 24px;
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
