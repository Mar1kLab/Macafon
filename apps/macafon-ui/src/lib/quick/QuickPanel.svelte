<script lang="ts">
  // Quick panel: the microphone switches, the preset, the Master and every channel that is on,
  // in one small window above the tray (Windows 11 flyout) with Android-like tiles and pill
  // sliders. The Windows tray opens it on right-click; Esc, the trigger again or a click elsewhere
  // closes it. It only talks to the daemon, like the main window.
  import { invoke } from '@tauri-apps/api/core';
  import {
    AudioLines,
    ChevronLeft,
    ChevronRight,
    Mic,
    MicOff,
    SlidersHorizontal,
    Speech,
    SquareArrowOutUpRight,
    Volume2,
    VolumeX,
    X,
    Power,
  } from '@lucide/svelte';
  import PillSlider from './PillSlider.svelte';
  import Logo from '../components/Logo.svelte';
  import { channelIcon } from '../mixer/icons';
  import { displayShortcut } from '../settings/shortcut';
  import { isTauri } from '../ipc/tauri';
  import { daemon } from '../state';
  import { dictationTile } from '../dictation/tile';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';

  let { onclose }: { onclose: () => void } = $props();
  const tauri = isTauri();
  let quitError = $state('');
  let quitting = $state(false);

  async function quit() {
    quitting = true;
    quitError = '';
    try {
      await daemon.client.call('daemon.quit');
    } catch (error) {
      quitError = String(error);
      quitting = false;
    }
  }

  const mic = $derived(daemon.mic);
  const mixer = $derived(daemon.mixer);
  const muted = $derived(mic?.settings.muted ?? false);
  const bypass = $derived(mic?.settings.bypass ?? false);
  const channels = $derived(mixer?.channels.filter((channel) => channel.enabled) ?? []);
  const preset = $derived(daemon.presets.find((p) => p.id === mic?.preset)?.name ?? t('quick.custom'));
  const output = $derived(mixer?.outputs.find((o) => o.name === mixer.output)?.description ?? null);
  const shortcut = $derived(daemon.hotkeys?.bindings.find((b) => b.action === 'quick_panel')?.shortcut ?? null);
  /** The look picked for the hotkey popup: the tray panel follows it, like the overlay (HK-006). */
  const sketch = $derived(daemon.sketch);
  const dictation = $derived(dictationTile(daemon.dictation, daemon.hotkeys));

  /** §9.9.2: toggles dictation; without a model, or on right click, the app opens instead. */
  function toggleDictation() {
    if (!dictation || dictation.needsModel) return openMain();
    daemon.setDictationPrefs({ enabled: !dictation.on });
  }

  function openMain() {
    if (tauri) void invoke('open_main').catch((error) => (quitError = String(error)));
  }

</script>

<div class="panel" class:sketch>
  {#if sketch}
    <!-- The overlay's frame: straight lines along the edges that run past the corners. -->
    <span class="frame" aria-hidden="true"><i class="t"></i><i class="b"></i><i class="l"></i><i class="r"></i></span>
  {/if}
  <header>
    <span class="brand"><span class="logo"><Logo label="" inactive={muted} bypass={bypass} /></span><b>Macafon</b></span>
    <button class="round" aria-label={t('quick.openApp')} {@attach tip(t('quick.openApp'))} onclick={openMain}>
      <SquareArrowOutUpRight size={15} strokeWidth={1.5} />
    </button>
    <button class="round" aria-label={t('quick.trayClose')} {@attach tip(t('quick.trayClose'))} onclick={onclose}>
      <X size={15} strokeWidth={1.5} />
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
        {#if muted}<MicOff size={15} strokeWidth={1.5} />{:else}<Mic size={15} strokeWidth={1.5} />{/if}
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
        <AudioLines size={15} strokeWidth={1.5} />
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
        <SlidersHorizontal size={15} strokeWidth={1.5} />
        <span class="title">{t('quick.mixer')}</span>
        <span class="state">{mixer?.enabled ? t('quick.on') : t('quick.off')}</span>
      </button>
      {#if dictation}
        <button
          class="tile"
          class:on={dictation.on}
          class:dim={dictation.needsModel}
          role="switch"
          aria-checked={dictation.on}
          {@attach tip(t('quick.dictationHint'))}
          onclick={toggleDictation}
          oncontextmenu={(event) => {
            event.preventDefault();
            openMain();
          }}
        >
          <Speech size={15} strokeWidth={1.5} />
          <span class="title">{t('quick.dictation')}</span>
          <span class="state">{dictation.state}</span>
          {#if dictation.progress !== null}<span class="progress" style:width="{dictation.progress}%"></span>{/if}
        </button>
      {/if}
    </div>

    <div class="preset">
      <span class="hud-label">{t('quick.preset')}</span>
      <button class="round" aria-label={t('quick.previousPreset')} {@attach tip(t('quick.previousPreset'))} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: -1 }))}>
        <ChevronLeft size={15} strokeWidth={1.5} />
      </button>
      <span class="name" aria-live="polite">{preset}{#if mic.preset_modified}<small>{t('quick.modified')}</small>{/if}</span>
      <button class="round" aria-label={t('quick.nextPreset')} {@attach tip(t('quick.nextPreset'))} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: 1 }))}>
        <ChevronRight size={15} strokeWidth={1.5} />
      </button>
    </div>

    {#if mixer?.enabled}
      <!-- Many channels on a small screen: the volumes scroll, the footer stays. -->
      <div class="volumes" style:max-height="{Math.max(160, screen.availHeight - 340)}px">
        <div class="row">
          <button
            class="round icon"
            class:alert={mixer.master_muted}
            aria-pressed={mixer.master_muted}
            aria-label={t('quick.mute', { name: t('mixer.master') })}
            {@attach tip(t('quick.mute', { name: t('mixer.master') }))}
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
            {sketch}
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
              {@attach tip(t('quick.mute', { name: channel.name }))}
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
            {sketch}
              onchange={(value) => daemon.editChannel(channel.id, (s) => (s.volume_pct = value))}
            />
          </div>
        {/each}
      </div>
    {:else}
      <p class="status">{t('quick.mixerOff')}</p>
    {/if}
  {/if}

  <button class="quit" disabled={quitting || !daemon.connected} onclick={quit}>
    <Power size={15} strokeWidth={1.5} />
    {t('quick.trayQuit')}
  </button>
  {#if quitError}<p class="status" role="alert">{quitError}</p>{/if}

  <footer>
    <span class="output" {@attach tip(output ?? '')}>{output ?? ''}</span>
    {#if shortcut}<kbd {@attach tip(t('quick.hotkey'))}>{displayShortcut(shortcut)}</kbd>{/if}
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
    gap: var(--space-2);
    padding: 12px;
    background: var(--bg-raised);
  }

  header,
  footer,
  .preset,
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
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
    gap: var(--space-2);
    margin-right: auto;
    font-family: var(--font-display);
    font-size: var(--text-md);
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
    width: 28px;
    height: 28px;
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

  .quit:focus-visible,
  .round:focus-visible,
  .tile:focus-visible {
    outline: 1px solid var(--ink);
    outline-offset: 2px;
  }

  .round.icon {
    width: 32px;
    height: 32px;
  }

  .round.channel {
    color: var(--c-ink);
  }

  .round.alert {
    color: var(--danger);
    border-color: var(--danger);
  }

  .quit {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: 6px 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--ink);
    font: inherit;
    font-size: var(--text-sm);
    cursor: pointer;
  }

  .quit:hover { border-color: var(--line-strong); }
  .quit:disabled { opacity: 0.5; cursor: default; }

  .tiles {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .tile {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    grid-template-rows: auto auto;
    align-items: center;
    column-gap: var(--space-2);
    min-width: 0;
    padding: 7px 10px;
    border-radius: 12px;
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

  .tile {
    position: relative;
    overflow: hidden;
  }

  .tile.dim {
    opacity: 0.6;
  }

  .tile .progress {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    background: var(--accent-ink);
  }

  .tile:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .tile > :global(svg) {
    grid-row: span 2;
  }

  .title {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    font-weight: 600;
    color: inherit;
  }

  .state {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-xs);
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
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--ink-strong);
  }

  .preset small {
    margin-left: 6px;
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    color: var(--app-amber-ink);
  }

  .volumes {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
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
    font-size: var(--text-md);
    color: var(--muted);
  }

  footer {
    justify-content: space-between;
    padding-top: 10px;
    border-top: 1px solid var(--line);
    font-size: var(--text-sm);
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
    font-size: var(--text-xs);
    color: var(--muted);
  }

  /* Sketch: the app's own look, picked with the hotkey popup's — sharp corners, HUD labels,
     accent marks instead of fills, and the overlay's frame. */
  .sketch {
    position: relative;
    isolation: isolate;
    padding: 24px;
    /* Painted only inside its lines; outside them the window is transparent. */
    background: transparent;
  }

  .sketch::before {
    content: '';
    position: absolute;
    inset: 8px;
    z-index: -1;
    background: var(--bg-raised);
  }

  .frame {
    position: absolute;
    inset: 8px;
    pointer-events: none;
  }

  .frame i {
    position: absolute;
    background: var(--line-strong);
  }

  .frame .t,
  .frame .b {
    left: -6px;
    right: -6px;
    height: 1px;
  }

  .frame .l,
  .frame .r {
    top: -6px;
    bottom: -6px;
    width: 1px;
  }

  .frame .t { top: 0; }
  .frame .b { bottom: 0; }
  .frame .l { left: 0; }
  .frame .r { right: 0; }

  .sketch header {
    padding-bottom: 12px;
    border-bottom: 1px solid var(--line);
  }

  .sketch .round,
  .sketch .tile,
  .sketch kbd {
    border-radius: 0;
  }

  .sketch .round {
    color: var(--muted);
  }

  .sketch .round:hover {
    color: var(--ink-strong);
  }

  .sketch .round.channel {
    color: var(--c-ink);
  }

  .sketch .round.alert {
    color: var(--danger);
  }

  .sketch .tile {
    position: relative;
  }

  .sketch .tile.on {
    background: var(--bg-sunken);
    border-color: var(--line);
    color: var(--ink-strong);
  }

  .sketch .tile:hover {
    background: var(--bg);
  }

  .sketch .tile.on :global(svg) {
    color: var(--accent-ink);
  }

  .sketch .tile.on::before,
  .sketch .tile.alert::before {
    content: '';
    position: absolute;
    left: -1px;
    top: -1px;
    bottom: -1px;
    width: 2px;
    background: var(--accent);
  }

  .sketch .tile.alert {
    border-color: var(--line);
  }

  .sketch .tile.alert::before {
    background: var(--danger);
  }

  .sketch .tile.alert .state {
    color: var(--danger);
  }

  .sketch .state {
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
    opacity: 1;
  }

  .sketch .quit {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.15em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .sketch .quit:hover:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }

  .sketch kbd {
    padding: 4px 8px;
    border: 1px solid var(--line);
  }

  .sketch footer {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  @media (prefers-reduced-motion: reduce) {
    .round,
    .tile {
      transition: none;
    }
  }
</style>
