<script lang="ts">
  // Quick panel: the microphone switches, the preset, the Master and every channel that is on,
  // in one small window above the tray (Windows 11 flyout) with Android-like tiles and pill
  // sliders. The daemon opens it on its hotkey; Esc, the hotkey again or a click elsewhere
  // closes it. It only talks to the daemon, like the main window.
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import {
    AudioLines,
    ChevronLeft,
    ChevronRight,
    Mic,
    MicOff,
    SlidersHorizontal,
    SquareArrowOutUpRight,
    Volume2,
    VolumeX,
  } from '@lucide/svelte';
  import PillSlider from './PillSlider.svelte';
  import Logo from '../components/Logo.svelte';
  import { channelIcon } from '../mixer/icons';
  import { displayShortcut } from '../settings/shortcut';
  import { isTauri } from '../ipc/tauri';
  import { daemon } from '../state';
  import { t } from '../i18n';

  let root: HTMLDivElement | undefined = $state();
  const tauri = isTauri();

  const mic = $derived(daemon.mic);
  const mixer = $derived(daemon.mixer);
  const muted = $derived(mic?.settings.muted ?? false);
  const bypass = $derived(mic?.settings.bypass ?? false);
  const channels = $derived(mixer?.channels.filter((channel) => channel.enabled) ?? []);
  const preset = $derived(daemon.presets.find((p) => p.id === mic?.preset)?.name ?? t('quick.custom'));
  const output = $derived(mixer?.outputs.find((o) => o.name === mixer.output)?.description ?? null);
  const shortcut = $derived(daemon.hotkeys?.bindings.find((b) => b.action === 'quick_panel')?.shortcut ?? null);

  function close() {
    if (tauri) void invoke('quick_panel_close');
  }

  function openMain() {
    if (tauri) void invoke('open_main');
  }

  onMount(() => {
    const onkey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    window.addEventListener('keydown', onkey);
    // The window takes the height of the content and shows itself once it knows it. Windows
    // gives an undecorated window with a shadow a taller viewport than asked (its hidden
    // caption): what the viewport gets beyond the request is taken off the next one.
    let extra = 0;
    const content = () => Math.ceil(root?.getBoundingClientRect().height ?? 0);
    const fit = () => {
      if (root && tauri) void invoke('quick_panel_fit', { height: content() - extra });
    };
    const onresize = () => {
      const off = window.innerHeight - content();
      if (Math.abs(off) <= 1) return;
      extra += off;
      fit();
    };
    const observer = new ResizeObserver(fit);
    if (root) observer.observe(root);
    window.addEventListener('resize', onresize);
    return () => {
      window.removeEventListener('keydown', onkey);
      window.removeEventListener('resize', onresize);
      observer.disconnect();
    };
  });
</script>

<div class="panel" bind:this={root}>
  <header>
    <span class="brand"><span class="logo"><Logo label="" inactive={muted} bypass={bypass} /></span><b>Macafon</b></span>
    <button class="round" aria-label={t('quick.openApp')} title={t('quick.openApp')} onclick={openMain}>
      <SquareArrowOutUpRight size={15} strokeWidth={1.5} />
    </button>
  </header>

  {#if !daemon.connected || !mic}
    <p class="status" role="status">
      {daemon.status.state === 'unavailable' ? t('daemon.unavailable') : t('daemon.loading')}
    </p>
  {:else}
    <div class="tiles">
      <button
        class="tile"
        class:on={!muted}
        class:alert={muted}
        role="switch"
        aria-checked={!muted}
        onclick={() => daemon.setMic(daemon.client.call('mic.set_muted', { muted: !muted }))}
      >
        {#if muted}<MicOff size={18} strokeWidth={1.5} />{:else}<Mic size={18} strokeWidth={1.5} />{/if}
        <span class="title">{t('quick.microphone')}</span>
        <span class="state">{muted ? t('quick.muted') : t('quick.on')}</span>
      </button>
      <button
        class="tile"
        class:on={!bypass}
        role="switch"
        aria-checked={!bypass}
        onclick={() => daemon.setMic(daemon.client.call('mic.set_bypass', { bypass: !bypass }))}
      >
        <AudioLines size={18} strokeWidth={1.5} />
        <span class="title">{t('quick.processing')}</span>
        <span class="state">{bypass ? t('quick.bypassed') : t('quick.on')}</span>
      </button>
      <button
        class="tile"
        class:on={mixer?.enabled}
        role="switch"
        aria-checked={mixer?.enabled ?? false}
        disabled={!mixer}
        onclick={() => mixer && daemon.setMixer(daemon.client.call('mixer.set_enabled', { enabled: !mixer.enabled }))}
      >
        <SlidersHorizontal size={18} strokeWidth={1.5} />
        <span class="title">{t('quick.mixer')}</span>
        <span class="state">{mixer?.enabled ? t('quick.on') : t('quick.off')}</span>
      </button>
    </div>

    <div class="preset">
      <span class="hud-label">{t('quick.preset')}</span>
      <button class="round" aria-label={t('quick.previousPreset')} title={t('quick.previousPreset')} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: -1 }))}>
        <ChevronLeft size={15} strokeWidth={1.5} />
      </button>
      <span class="name" aria-live="polite">{preset}{#if mic.preset_modified}<small>{t('quick.modified')}</small>{/if}</span>
      <button class="round" aria-label={t('quick.nextPreset')} title={t('quick.nextPreset')} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: 1 }))}>
        <ChevronRight size={15} strokeWidth={1.5} />
      </button>
    </div>

    {#if mixer?.enabled}
      <div class="volumes">
        <div class="row">
          <button
            class="round icon"
            class:alert={mixer.master_muted}
            aria-pressed={mixer.master_muted}
            aria-label={t('quick.mute', { name: t('mixer.master') })}
            title={t('quick.mute', { name: t('mixer.master') })}
            onclick={() => daemon.setMaster({ muted: !mixer.master_muted })}
          >
            {#if mixer.master_muted}<VolumeX size={16} strokeWidth={1.5} />{:else}<Volume2 size={16} strokeWidth={1.5} />{/if}
          </button>
          <PillSlider
            value={mixer.master_volume_pct}
            name={t('mixer.master')}
            label={t('mixer.masterVolume')}
            color="teal"
            muted={mixer.master_muted}
            mutedText={t('quick.muted')}
            onchange={(volume_pct) => daemon.setMaster({ volume_pct })}
          />
        </div>
        <hr />
        {#each channels as channel (channel.id)}
          {@const Icon = channelIcon(channel.icon)}
          <div class="row" style:--c-ink="var(--app-{channel.color}-ink)">
            <button
              class="round icon channel"
              class:alert={channel.settings.muted}
              aria-pressed={channel.settings.muted}
              aria-label={t('quick.mute', { name: channel.name })}
              title={t('quick.mute', { name: channel.name })}
              onclick={() => daemon.editChannel(channel.id, (s) => (s.muted = !s.muted))}
            >
              {#if channel.settings.muted}<VolumeX size={16} strokeWidth={1.5} />{:else}<Icon size={16} strokeWidth={1.5} />{/if}
            </button>
            <PillSlider
              value={channel.settings.volume_pct}
              name={channel.name}
              label={t('mixer.volume', { name: channel.name })}
              color={channel.color}
              muted={channel.settings.muted}
              mutedText={t('quick.muted')}
              onchange={(value) => daemon.editChannel(channel.id, (s) => (s.volume_pct = value))}
            />
          </div>
        {/each}
      </div>
    {:else}
      <p class="status">{t('quick.mixerOff')}</p>
    {/if}
  {/if}

  <footer>
    <span class="output" title={output ?? ''}>{output ?? ''}</span>
    {#if shortcut}<kbd title={t('quick.hotkey')}>{displayShortcut(shortcut)}</kbd>{/if}
  </footer>
</div>

<style>
  :global(html),
  :global(body) {
    background: var(--bg-raised);
    overflow: hidden;
  }

  .panel {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px;
    background: var(--bg-raised);
  }

  header,
  footer,
  .preset,
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .logo {
    display: inline-flex;
    width: 20px;
    height: 20px;
  }

  .logo :global(svg) {
    width: 100%;
    height: 100%;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-right: auto;
    font-family: var(--font-display);
    font-size: 14px;
    letter-spacing: -0.02em;
    color: var(--ink-strong);
  }

  /* Round controls and tiles: the Android/Windows 11 look the owner asked for (exception to
     the sharp corners of the main window, like the hotkey popup). */
  .round {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--ink);
    cursor: pointer;
    transition:
      border-color var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  .round:hover {
    border-color: var(--line-strong);
  }

  .round:focus-visible,
  .tile:focus-visible {
    outline: 1px solid var(--ink);
    outline-offset: 2px;
  }

  .round.icon {
    width: 40px;
    height: 40px;
  }

  .round.channel {
    color: var(--c-ink);
  }

  .round.alert {
    color: var(--danger);
    border-color: var(--danger);
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }

  .tile {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    min-width: 0;
    padding: 12px;
    border-radius: 16px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--ink);
    text-align: left;
    cursor: pointer;
    transition:
      background var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  .tile.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--ink-strong);
  }

  .tile.alert {
    border-color: var(--danger);
    color: var(--danger);
  }

  .tile:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .title {
    margin-top: 4px;
    font-size: 13px;
    font-weight: 600;
    color: inherit;
  }

  .state {
    font-size: 12px;
    opacity: 0.8;
  }

  .preset .hud-label {
    margin-right: auto;
  }

  .preset .name {
    min-width: 120px;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: center;
    font-size: 13px;
    font-weight: 600;
    color: var(--ink-strong);
  }

  .preset small {
    margin-left: 6px;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--app-amber-ink);
  }

  .volumes {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .row :global(.pill) {
    flex: 1;
    min-width: 0;
  }

  hr {
    margin: 2px 0;
    border: 0;
    border-top: 1px solid var(--line);
  }

  .status {
    padding: 8px 0;
    font-size: 13px;
    color: var(--muted);
  }

  footer {
    justify-content: space-between;
    padding-top: 10px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--faint);
  }

  .output {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  kbd {
    flex: none;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--muted);
  }

  @media (prefers-reduced-motion: reduce) {
    .round,
    .tile {
      transition: none;
    }
  }
</style>
