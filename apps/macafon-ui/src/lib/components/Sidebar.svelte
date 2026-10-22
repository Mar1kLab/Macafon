<script lang="ts" module>
  export type Screen = 'microphone' | 'mixer' | 'soundpad' | 'dictation' | 'displays' | 'settings';
</script>

<script lang="ts">
  // 64px icon sidebar (PRD §9.1) with HUD labels on hover; daemon status at the bottom.
  import { AudioWaveform, Mic, MonitorSmartphone, Settings, SlidersVertical, Speech } from '@lucide/svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import Logo from './Logo.svelte';
  import { tip } from './tooltip.svelte';

  let { screen, onnavigate }: { screen: Screen; onnavigate: (screen: Screen) => void } = $props();

  const items = $derived([
    { id: 'microphone' as const, icon: Mic, label: t('nav.microphone') },
    { id: 'mixer' as const, icon: SlidersVertical, label: t('nav.mixer') },
    ...(daemon.soundpadAvailable ? [{ id: 'soundpad' as const, icon: AudioWaveform, label: t('nav.soundpad') }] : []),
    ...(daemon.dictationAvailable ? [{ id: 'dictation' as const, icon: Speech, label: t('nav.dictation') }] : []),
    ...(daemon.displaysAvailable ? [{ id: 'displays' as const, icon: MonitorSmartphone, label: t('nav.displays') }] : []),
  ]);

  const status = $derived(daemon.status);
  const statusText = $derived(
    status.state === 'connected'
      ? daemon.mock
        ? t('daemon.mock')
        : t('daemon.connected', { version: status.version })
      : status.state === 'connecting'
        ? t('daemon.connecting')
        : t('daemon.unavailable'),
  );
  const muted = $derived(daemon.mic?.settings.muted ?? false);
  const bypass = $derived(daemon.mic?.settings.bypass ?? false);
  const error = $derived(status.state === 'unavailable' || daemon.mic?.input_connected === false);
  const logoLabel = $derived([
    t('app.name'),
    ...(muted ? [t('mic.muted')] : []),
    ...(bypass ? [t('mic.bypass')] : []),
    ...(error ? [t('mic.disconnected')] : []),
    ...(!muted && !bypass && !error ? [status.state === 'connecting' ? t('daemon.loading') : t('mic.active')] : []),
  ].join(' · '));
</script>

<nav aria-label="Macafon">
  <div class="logo" {@attach tip(logoLabel)}>
    <Logo inactive={muted || bypass || error} {bypass} {error} label={logoLabel} />
  </div>
  <ul>
    {#each items as item (item.id)}
      {@const Icon = item.icon}
      <li>
        <button
          class:active={screen === item.id}
          aria-current={screen === item.id ? 'page' : undefined}
          aria-label={item.label}
          onclick={() => onnavigate(item.id)}
        >
          <Icon size={20} strokeWidth={1.5} />
          <span class="tip hud-label">{item.label}</span>
        </button>
      </li>
    {/each}
  </ul>
  <div class="bottom">
    <button class:active={screen === 'settings'} aria-current={screen === 'settings' ? 'page' : undefined} aria-label={t('nav.settings')} onclick={() => onnavigate('settings')}><Settings size={20} strokeWidth={1.5} /><span class="tip hud-label">{t('nav.settings')}</span></button>
  <div class="status" {@attach tip(statusText)} role="status" aria-label={statusText}>
    {#if muted}<span class="badge muted">{t('mic.muted')}</span>{/if}
    <span class="dot {status.state}" class:mock={daemon.mock}></span>
    <span class="status-label">{status.state === 'connected' ? t('daemon.running') : status.state === 'connecting' ? t('daemon.loading') : t('daemon.stopped')}</span>
  </div>
  </div>
</nav>

<style>
  nav {
    width: var(--sidebar-width);
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    border-right: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .logo {
    height: 58px;
    display: flex;
    align-items: center;
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
    border-bottom: 1px solid var(--line);
    width: 100%;
    justify-content: center;
  }

  ul {
    list-style: none;
    margin: 12px 0 0;
    padding: 0;
    display: grid;
    gap: var(--space-1);
  }

  button {
    position: relative;
    width: 44px;
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--muted);
    border: 1px solid transparent;
    transition:
      color var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  button:hover:not(:disabled) {
    color: var(--ink);
    border-color: var(--line);
  }

  button.active {
    color: var(--ink-strong);
    border-color: var(--line-strong);
    background: var(--bg-sunken);
  }

  .tip {
    position: absolute;
    left: calc(100% + 12px);
    z-index: 40;
    padding: 4px 8px;
    background: var(--bg-sunken);
    border: 1px solid var(--line);
    color: var(--ink);
    white-space: nowrap;
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--dur) var(--ease);
  }

  button:hover .tip,
  button:focus-visible .tip {
    opacity: 1;
  }

  .bottom { margin-top: auto; display: grid; justify-items: center; width: 100%; }
  .status-label { font-family: var(--font-mono); font-size: var(--text-2xs); color: var(--muted); }
  .status {
    padding: 16px 0;
    display: grid;
    justify-items: center;
    gap: var(--space-2);
  }

  .muted {
    color: var(--danger);
    border-color: var(--danger);
    padding: 1px 3px;
    font-size: var(--text-2xs);
  }

  .dot {
    width: 8px;
    height: 8px;
    background: var(--faint);
  }

  .dot.connected {
    background: var(--app-teal-ink);
  }

  .dot.connected.mock {
    background: var(--app-amber-ink);
  }

  .dot.unavailable {
    background: var(--danger);
  }
</style>
