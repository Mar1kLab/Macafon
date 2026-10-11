<script lang="ts">
  // Channel strip (design system §6, PRD §9.4): colour bar, name, app chips (a drop target),
  // fader with a stereo meter, % and dB, M / EQ / FX. While a chip is dragged over it the
  // strip lights up in its colour with corner ticks; a new app flashes in.
  import { Settings2 } from '@lucide/svelte';
  import AppChip from './AppChip.svelte';
  import Fader from './Fader.svelte';
  import StereoMeter from './StereoMeter.svelte';
  import Ticks from '../components/Ticks.svelte';
  import { channelIcon } from './icons';
  import { daemon } from '../state';
  import { formatVolumeDb } from '../dsp/format';
  import { t } from '../i18n';
  import type { AppState, ChannelState } from '../ipc/protocol';

  interface Props {
    channel: ChannelState;
    channels: ChannelState[];
    apps: AppState[];
    isDefault: boolean;
    dragging: string | null;
    flashing: Set<string>;
    onpanel: (panel: 'eq' | 'fx' | 'channel') => void;
    ondragstart: (key: string) => void;
    ondragend: () => void;
  }

  let { channel, channels, apps, isDefault, dragging, flashing, onpanel, ondragstart, ondragend }: Props = $props();
  let over = $state(false);
  const s = $derived(channel.settings);
  const Icon = $derived(channelIcon(channel.icon));
  const eqOn = $derived(s.eq.enabled && !s.bypass);
  const fxOn = $derived(!s.bypass && (s.noise_suppression.enabled || s.compressor.enabled || s.limiter.enabled));
  const target = $derived(dragging !== null && !apps.some((app) => app.key === dragging));

  function move(key: string, to: string) {
    void daemon.setMixer(daemon.client.call('apps.move', { app_key: key, channel: to }));
  }

  function ondrop(event: DragEvent) {
    event.preventDefault();
    over = false;
    const key = event.dataTransfer?.getData('application/x-macafon-app');
    if (key && !apps.some((app) => app.key === key)) move(key, channel.id);
  }
</script>

<section
  class="strip"
  class:over={over && target}
  class:target
  aria-label={channel.name}
  style:--c="var(--app-{channel.color})"
  style:--c-ink="var(--app-{channel.color}-ink)"
  ondragover={(event) => {
    if (!target) return;
    event.preventDefault();
    over = true;
  }}
  ondragleave={(event) => {
    if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node)) over = false;
  }}
  {ondrop}
>
  {#if over && target}<span class="ticks"><Ticks color="var(--c-ink)" /></span>{/if}
  <header>
    <Icon size={14} strokeWidth={1.5} />
    <h2>{channel.name}</h2>
    {#if isDefault}<span class="badge">{t('mixer.default')}</span>{/if}
    <button class="btn icon ghost small settings" aria-label={t('mixer.channelSettings', { name: channel.name })} title={t('mixer.channelSettings', { name: channel.name })} onclick={() => onpanel('channel')}>
      <Settings2 size={13} strokeWidth={1.5} />
    </button>
  </header>
  <ul class="apps" aria-label={t('mixer.appsIn', { name: channel.name })}>
    {#each apps as app (app.key)}
      <li class:flash={flashing.has(app.key)}>
        <AppChip
          {app}
          {channels}
          color={channel.color}
          onmove={(to) => move(app.key, to)}
          onmanaged={(managed) => daemon.setMixer(daemon.client.call('apps.set_managed', { app_key: app.key, managed }))}
          onforget={() => daemon.setMixer(daemon.client.call('apps.forget', { app_key: app.key }))}
          {ondragstart}
          {ondragend}
        />
      </li>
    {:else}
      <li class="empty">{t('mixer.dropHere')}</li>
    {/each}
  </ul>
  <div class="level">
    <Fader
      value={s.volume_pct}
      label={t('mixer.volume', { name: channel.name })}
      color={channel.color}
      defaultValue={100}
      onchange={(value) => daemon.editChannel(channel.id, (settings) => (settings.volume_pct = value))}
    />
    <StereoMeter id={channel.id} color={channel.color} />
  </div>
  <p class="readout">
    <span class="value">{Math.round(s.volume_pct)}%</span>
    <span class="db">{formatVolumeDb(s.volume_pct)}</span>
  </p>
  <footer>
    <button
      class="btn small danger"
      class:on={s.muted}
      aria-pressed={s.muted}
      title={t('mixer.mute')}
      onclick={() => daemon.editChannel(channel.id, (settings) => (settings.muted = !settings.muted))}>M</button
    >
    <button class="btn small" title={t('mixer.eqHint')} onclick={() => onpanel('eq')}>
      EQ{#if eqOn}<span class="active-dot" aria-label={t('mixer.active')}></span>{/if}
    </button>
    <button class="btn small" title={t('mixer.fxHint')} onclick={() => onpanel('fx')}>
      FX{#if fxOn}<span class="active-dot" aria-label={t('mixer.active')}></span>{/if}
    </button>
  </footer>
</section>

<style>
  .strip {
    position: relative;
    display: grid;
    grid-template-rows: auto minmax(84px, 1fr) minmax(160px, 260px) auto auto;
    gap: 12px;
    width: 184px;
    flex: none;
    padding: 0 12px 12px;
    border: 1px solid var(--line);
    border-top: 3px solid var(--c);
    background: var(--bg-raised);
    transition: border-color var(--dur) var(--ease);
  }

  .strip.over {
    border-color: var(--c);
  }

  .ticks {
    position: absolute;
    inset: -3px 0 0;
    pointer-events: none;
  }

  header {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    color: var(--c-ink);
  }

  h2 {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-display);
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
  }

  .settings {
    margin-left: auto;
  }

  .apps {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }

  .empty {
    padding: 8px 0;
    font-size: 12px;
    color: var(--faint);
  }

  .target .empty {
    color: var(--c-ink);
  }

  .flash :global(.chip) {
    animation: flash 900ms ease-out;
  }

  @keyframes flash {
    from {
      background: color-mix(in srgb, var(--c) 45%, transparent);
      border-color: var(--c);
    }
  }

  .level {
    display: flex;
    justify-content: center;
    gap: 14px;
    min-height: 0;
  }

  .readout {
    display: flex;
    justify-content: center;
    align-items: baseline;
    gap: 8px;
  }

  .readout .value {
    font-family: var(--font-display);
    font-size: 18px;
    font-weight: 600;
    color: var(--ink-strong);
  }

  .db {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--muted);
  }

  footer {
    display: flex;
    gap: 6px;
  }

  footer .btn {
    flex: 1;
    justify-content: center;
  }

  .active-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--c-ink);
  }

  @media (prefers-reduced-motion: reduce) {
    .flash :global(.chip) {
      animation: none;
    }
  }
</style>
