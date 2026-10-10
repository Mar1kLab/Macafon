<script lang="ts" module>
  // The Mixer screen joins in M2 (PRD §9.4).
  export type Screen = 'microphone' | 'settings';
</script>

<script lang="ts">
  // 64px icon sidebar (PRD §9.1) with HUD labels on hover; daemon status at the bottom.
  import { Mic, Settings } from '@lucide/svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';

  let { screen, onnavigate }: { screen: Screen; onnavigate: (screen: Screen) => void } = $props();

  const items = [
    { id: 'microphone' as const, icon: Mic, label: t('nav.microphone') },
    { id: 'settings' as const, icon: Settings, label: t('nav.settings') },
  ];

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
</script>

<nav aria-label="Macafon">
  <div class="logo" title={t('app.name')}>
    <span class="mark" aria-hidden="true">M</span>
    <span class="visually-hidden">{t('app.name')}</span>
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
  <div class="status" title={statusText} role="status" aria-label={statusText}>
    {#if muted}<span class="badge muted">{t('mic.muted')}</span>{/if}
    <span class="dot {status.state}" class:mock={daemon.mock}></span>
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
    font-size: 15px;
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
    border-bottom: 1px solid var(--line);
    width: 100%;
    justify-content: center;
  }

  /* Monogram: the full name does not fit 64px; the accent bar echoes a level meter. */
  .mark {
    position: relative;
    font-size: 22px;
    padding-bottom: 3px;
  }

  .mark::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    background: var(--accent);
  }

  ul {
    list-style: none;
    margin: 12px 0 0;
    padding: 0;
    display: grid;
    gap: 4px;
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

  .status {
    margin-top: auto;
    padding: 16px 0;
    display: grid;
    justify-items: center;
    gap: 10px;
  }

  .muted {
    color: var(--danger);
    border-color: var(--danger);
    padding: 1px 3px;
    font-size: 8px;
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
