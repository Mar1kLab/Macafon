<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { AudioLines, ChevronLeft, ChevronRight, Mic, MicOff, SlidersHorizontal, Volume2, VolumeX, X } from '@lucide/svelte';
  import Logo from '../components/Logo.svelte';
  import VerticalSlider from './VerticalSlider.svelte';
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
  const preset = $derived(daemon.presets.find((item) => item.id === mic?.preset)?.name ?? t('quick.custom'));
  const shortcut = $derived(daemon.hotkeys?.bindings.find((binding) => binding.action === 'quick_panel')?.shortcut);

  function close() {
    if (tauri) void invoke('quick_panel_close');
  }

  onMount(() => {
    const onkey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); close(); }
    };
    window.addEventListener('keydown', onkey);
    // Undecorated Windows viewports can exceed the requested size in either dimension.
    // Keep the content independent of the viewport so correction converges after resize.
    let extraWidth = 0;
    let extraHeight = 0;
    const content = () => ({
      width: Math.ceil(root?.getBoundingClientRect().width ?? 0),
      height: Math.ceil(root?.getBoundingClientRect().height ?? 0),
    });
    const fit = () => {
      if (!root || !tauri) return;
      const { width, height } = content();
      void invoke('quick_panel_fit', { width: width - extraWidth, height: height - extraHeight });
    };
    const onresize = () => {
      if (!tauri) return;
      const { width, height } = content();
      const offWidth = window.innerWidth - width;
      const offHeight = window.innerHeight - height;
      if (Math.abs(offWidth) <= 1 && Math.abs(offHeight) <= 1) return;
      extraWidth += offWidth;
      extraHeight += offHeight;
      fit();
    };
    const observer = new ResizeObserver(fit);
    if (root) observer.observe(root);
    window.addEventListener('resize', onresize);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', onresize);
      window.removeEventListener('keydown', onkey);
    };
  });
</script>

<div class="overlay" bind:this={root}>
  <header>
    <span class="brand"><span class="logo"><Logo label="" inactive={muted} {bypass} /></span><b>{t('app.name')}</b></span>
    <span class="hud-label">{t('quick.overlayTitle')}</span>
    <span class="header-end">
      {#if shortcut}<kbd title={t('quick.hotkey')}>{displayShortcut(shortcut)}</kbd>{/if}
      <button class="round close" aria-label={t('quick.closeOverlay')} title={t('quick.closeOverlay')} onclick={close}><X size={18} strokeWidth={1.5} /></button>
    </span>
  </header>

  {#if !daemon.connected || !mic}
    <p class="status" role="status">{daemon.status.state === 'unavailable' ? t('daemon.unavailable') : t('daemon.loading')}</p>
  {:else}
    <div class="body">
      <section class="volumes" aria-label={t('quick.mixer')}>
        {#if mixer?.enabled}
          <div class="strip master" style:--channel-ink="var(--app-teal-ink)">
            <span class="strip-name" title={t('mixer.master')}>{t('mixer.master')}</span>
            <VerticalSlider value={mixer.master_volume_pct} label={t('mixer.masterVolume')} color="teal" muted={mixer.master_muted} onchange={(volume_pct) => daemon.setMaster({ volume_pct })} />
            <button class="round mute" class:alert={mixer.master_muted} aria-pressed={mixer.master_muted} aria-label={t('quick.mute', { name: t('mixer.master') })} title={t('quick.mute', { name: t('mixer.master') })} onclick={() => mixer && daemon.setMaster({ muted: !mixer.master_muted })}>
              {#if mixer.master_muted}<VolumeX size={18} strokeWidth={1.5} />{:else}<Volume2 size={18} strokeWidth={1.5} />{/if}
            </button>
          </div>
          {#each channels as channel (channel.id)}
            {@const Icon = channelIcon(channel.icon)}
            <div class="strip" style:--channel-ink="var(--app-{channel.color}-ink)">
              <span class="strip-name" title={channel.name}>{channel.name}</span>
              <VerticalSlider value={channel.settings.volume_pct} label={t('mixer.volume', { name: channel.name })} color={channel.color} muted={channel.settings.muted} onchange={(value) => daemon.editChannel(channel.id, (settings) => (settings.volume_pct = value))} />
              <button class="round mute" class:alert={channel.settings.muted} aria-pressed={channel.settings.muted} aria-label={t('quick.mute', { name: channel.name })} title={t('quick.mute', { name: channel.name })} onclick={() => daemon.editChannel(channel.id, (settings) => (settings.muted = !settings.muted))}>
                {#if channel.settings.muted}<VolumeX size={18} strokeWidth={1.5} />{:else}<Icon size={18} strokeWidth={1.5} />{/if}
              </button>
            </div>
          {/each}
        {:else}
          <p class="status" role="status">{t('quick.mixerOff')}</p>
        {/if}
      </section>

      <section class="controls" aria-label={t('quick.microphone')}>
        <button class="toggle" class:alert={muted} role="switch" aria-checked={!muted} onclick={() => daemon.setMic(daemon.client.call('mic.set_muted', { muted: !muted }))}>
          {#if muted}<MicOff size={18} strokeWidth={1.5} />{:else}<Mic size={18} strokeWidth={1.5} />{/if}
          <span>{t('quick.microphone')}</span><small>{muted ? t('quick.muted') : t('quick.on')}</small>
        </button>
        <button class="toggle" class:active={!bypass} role="switch" aria-checked={!bypass} onclick={() => daemon.setMic(daemon.client.call('mic.set_bypass', { bypass: !bypass }))}>
          <AudioLines size={18} strokeWidth={1.5} /><span>{t('quick.processing')}</span><small>{bypass ? t('quick.bypassed') : t('quick.on')}</small>
        </button>
        <button class="toggle" class:active={mixer?.enabled} role="switch" aria-checked={mixer?.enabled ?? false} disabled={!mixer} onclick={() => mixer && daemon.setMixer(daemon.client.call('mixer.set_enabled', { enabled: !mixer.enabled }))}>
          <SlidersHorizontal size={18} strokeWidth={1.5} /><span>{t('quick.mixer')}</span><small>{mixer?.enabled ? t('quick.on') : t('quick.off')}</small>
        </button>
        <div class="preset">
          <span class="hud-label">{t('quick.preset')}</span>
          <div class="preset-nav">
            <button class="round" aria-label={t('quick.previousPreset')} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: -1 }))}><ChevronLeft size={16} strokeWidth={1.5} /></button>
            <span class="preset-name" title={preset} aria-live="polite">{preset}{#if mic.preset_modified}<small>{t('quick.modified')}</small>{/if}</span>
            <button class="round" aria-label={t('quick.nextPreset')} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: 1 }))}><ChevronRight size={16} strokeWidth={1.5} /></button>
          </div>
        </div>
      </section>
    </div>
  {/if}
</div>

<style>
  :global(html), :global(body) { background: var(--bg-raised); background-image: none; overflow: hidden; }
  .overlay { width: 840px; min-height: 368px; padding: 20px 24px; border: 1px solid var(--line); border-radius: 24px; background: var(--bg-raised); }
  header { display: flex; align-items: center; gap: 24px; padding-bottom: 16px; border-bottom: 1px solid var(--line); }
  .brand { display: flex; align-items: center; gap: 10px; font-family: var(--font-display); font-size: 18px; letter-spacing: -0.03em; color: var(--ink-strong); }
  .logo { display: inline-flex; width: 24px; height: 28px; }
  .logo :global(svg) { width: 100%; height: 100%; }
  .header-end { margin-left: auto; display: flex; align-items: center; gap: 20px; }
  kbd { font-family: var(--font-mono); font-size: 11px; color: var(--muted); border: 1px solid var(--line); border-radius: 6px; padding: 4px 8px; }
  .round { width: 32px; height: 32px; display: inline-flex; align-items: center; justify-content: center; flex: none; border: 1px solid var(--line); border-radius: 50%; color: var(--muted); background: var(--bg-sunken); transition: border-color var(--dur) var(--ease), color var(--dur) var(--ease); }
  .round:hover { border-color: var(--line-strong); color: var(--ink-strong); }
  .close { background: var(--bg); }
  .body { display: flex; gap: 24px; padding-top: 16px; }
  .volumes { min-width: 0; flex: 1; display: flex; overflow-x: auto; padding: 0 2px 4px; }
  .strip { flex: 1 0 88px; min-width: 0; display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .master { border-right: 1px solid var(--line); margin-right: 8px; padding-right: 8px; }
  .strip-name { font-family: var(--font-display); font-size: 13px; font-weight: 600; color: var(--ink-strong); max-width: 88px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .mute { color: var(--channel-ink); }
  .alert, .round.alert { color: var(--danger); border-color: var(--danger); }
  .controls { width: 232px; flex: none; padding-left: 24px; border-left: 1px solid var(--line); display: flex; flex-direction: column; gap: 8px; }
  .toggle { display: flex; align-items: center; gap: 10px; min-height: 44px; padding: 8px 12px; border: 1px solid var(--line); border-radius: 12px; background: var(--bg-sunken); text-align: left; font-size: 12px; }
  .toggle small { margin-left: auto; color: var(--muted); font-size: 10px; }
  .toggle.active :global(svg) { color: var(--accent-ink); }
  .toggle.alert small { color: var(--danger); }
  .toggle:hover { background: var(--bg); }
  .toggle:disabled { opacity: 0.5; }
  .preset { margin-top: auto; padding-top: 8px; }
  .preset .hud-label { font-size: 9px; }
  .preset-nav { display: flex; gap: 8px; align-items: center; margin-top: 6px; }
  .preset-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: center; font-size: 12px; }
  .preset-name small { display: block; font-size: 9px; color: var(--app-amber-ink); }
  .status { flex: 1; align-self: center; padding: 24px; font-size: 13px; color: var(--muted); }
  @media (prefers-reduced-motion: reduce) { .round { transition: none; } }
</style>
