<script lang="ts">
  import { AudioLines, AudioWaveform, ChevronLeft, ChevronRight, Mic, MicOff, SlidersHorizontal, Speech, Square, Volume2, VolumeX, X } from '@lucide/svelte';
  import Logo from '../components/Logo.svelte';
  import VerticalSlider from './VerticalSlider.svelte';
  import SoundpadOverlay from './SoundpadOverlay.svelte';
  import DictationOverlay from './DictationOverlay.svelte';
  import { channelIcon } from '../mixer/icons';
  import { displayShortcut } from '../settings/shortcut';
  import { daemon } from '../state';
  import { dictationTile } from '../dictation/tile';
  import { t } from '../i18n';

  let { onclose, opened = 0 }: { onclose: () => void; opened?: number } = $props();
  const mic = $derived(daemon.mic);
  const mixer = $derived(daemon.mixer);
  const muted = $derived(mic?.settings.muted ?? false);
  const bypass = $derived(mic?.settings.bypass ?? false);
  const channels = $derived(mixer?.channels.filter((channel) => channel.enabled) ?? []);
  const preset = $derived(daemon.presets.find((item) => item.id === mic?.preset)?.name ?? t('quick.custom'));
  const shortcut = $derived(daemon.hotkeys?.bindings.find((binding) => binding.action === 'quick_panel')?.shortcut);
  const soundpad = $derived(daemon.soundpad?.prefs.show_in_overlay ? daemon.soundpad : null);
  const dictation = $derived(dictationTile(daemon.dictation, daemon.hotkeys));
  const playback = $derived(daemon.soundpadPlayback);
  /** The look picked for the hotkey popup: the overlay follows it (HK-006, HK-007). */
  const sketch = $derived(daemon.prefs?.osd.look === 'sketch');
  const playing = $derived(playback && soundpad?.sounds.find((sound) => sound.id === playback.sound_id));
  /** SND-006: the sounds open under the controls by their round button; the overlay opens
   * with them the way it was last closed, across openings and UI restarts. */
  const SOUNDS_KEY = 'macafon.overlay.soundsOpen';
  let sounds = $state(localStorage.getItem(SOUNDS_KEY) === '1');

  /** §9.9.2: the dictation panel opens the same way; one panel at a time. */
  const DICTATION_KEY = 'macafon.overlay.dictationOpen';
  let dictationOpen = $state(localStorage.getItem(DICTATION_KEY) === '1');
  const lastResult = $derived(daemon.dictation?.history[0] ?? null);

  function setSounds(open: boolean) {
    sounds = open;
    localStorage.setItem(SOUNDS_KEY, open ? '1' : '0');
    if (open) setDictationOpen(false);
  }

  function setDictationOpen(open: boolean) {
    dictationOpen = open;
    localStorage.setItem(DICTATION_KEY, open ? '1' : '0');
    if (open && sounds) setSounds(false);
  }

  // The page stays loaded between openings: each one restores what the last left.
  $effect(() => {
    void opened;
    sounds = localStorage.getItem(SOUNDS_KEY) === '1';
    dictationOpen = localStorage.getItem(DICTATION_KEY) === '1';
  });
</script>

<div class="overlay" class:sketch>
  <!-- A sketch-like frame: straight lines along the edges that run past the corners. -->
  <span class="frame" aria-hidden="true"><i class="t"></i><i class="b"></i><i class="l"></i><i class="r"></i></span>
  <header>
    <span class="brand"><span class="logo"><Logo label="" inactive={muted} {bypass} /></span><b>{t('app.name')}</b></span>
    <span class="hud-label">{t('quick.overlayTitle')}</span>
    <span class="header-end">
      {#if shortcut}<kbd title={t('quick.hotkey')}>{displayShortcut(shortcut)}</kbd>{/if}
      <button class="round close" aria-label={t('quick.closeOverlay')} title={t('quick.closeOverlay')} onclick={onclose}><X size={18} strokeWidth={1.5} /></button>
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
            <VerticalSlider {sketch} value={mixer.master_volume_pct} label={t('mixer.masterVolume')} color="teal" muted={mixer.master_muted} onchange={(volume_pct) => daemon.setMaster({ volume_pct })} />
            <button class="round mute" class:alert={mixer.master_muted} aria-pressed={mixer.master_muted} aria-label={t('quick.mute', { name: t('mixer.master') })} title={t('quick.mute', { name: t('mixer.master') })} onclick={() => mixer && daemon.setMaster({ muted: !mixer.master_muted })}>
              {#if mixer.master_muted}<VolumeX size={18} strokeWidth={1.5} />{:else}<Volume2 size={18} strokeWidth={1.5} />{/if}
            </button>
          </div>
          {#each channels as channel (channel.id)}
            {@const Icon = channelIcon(channel.icon)}
            <div class="strip" style:--channel-ink="var(--app-{channel.color}-ink)">
              <span class="strip-name" title={channel.name}>{channel.name}</span>
              <VerticalSlider {sketch} value={channel.settings.volume_pct} label={t('mixer.volume', { name: channel.name })} color={channel.color} muted={channel.settings.muted} onchange={(value) => daemon.editChannel(channel.id, (settings) => (settings.volume_pct = value))} />
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
        {#if dictation}
          <button class="toggle" class:active={dictation.on} role="switch" aria-checked={dictation.on} disabled={dictation.needsModel} title={t('quick.dictationHint')} onclick={() => daemon.setDictationPrefs({ enabled: !dictation.on })}>
            <Speech size={18} strokeWidth={1.5} /><span>{t('quick.dictation')}</span><small>{dictation.state}</small>
            {#if dictation.progress !== null}<i class="progress" style:width="{dictation.progress}%"></i>{/if}
          </button>
        {/if}
        {#if dictation && !dictation.needsModel}
          <div class="sounds">
            <button class="round sounds-toggle" class:open={dictationOpen} aria-expanded={dictationOpen} aria-controls="overlay-dictation" aria-label={dictationOpen ? t('quick.dictationHide') : t('quick.dictationShow')} title={dictationOpen ? t('quick.dictationHide') : t('quick.dictationShow')} onclick={() => setDictationOpen(!dictationOpen)}>
              <Speech size={16} strokeWidth={1.5} />
            </button>
            <span class="sounds-label">{#if lastResult}<b title={lastResult.text}>{lastResult.text}</b>{:else}{t('quick.dictationHistory')}<small>{daemon.dictation?.history.length ?? 0}</small>{/if}</span>
          </div>
        {/if}
        {#if soundpad}
          <div class="sounds">
            <button class="round sounds-toggle" class:open={sounds} aria-expanded={sounds} aria-controls="overlay-sounds" aria-label={sounds ? t('quick.soundpadHide') : t('quick.soundpadShowHint')} title={sounds ? t('quick.soundpadHide') : t('quick.soundpadShowHint')} onclick={() => setSounds(!sounds)}>
              <AudioWaveform size={16} strokeWidth={1.5} />
            </button>
            <span class="sounds-label">{#if playing}<b title={playing.name}>{playing.name}</b>{:else}{t('quick.soundpadShow')}<small>{soundpad.sounds.length}</small>{/if}</span>
            {#if playback && !sounds}
              <button class="round" aria-label={t('quick.soundpadStop')} title={t('quick.soundpadStop')} onclick={() => daemon.stopSound()}><Square size={12} strokeWidth={1.5} /></button>
            {/if}
          </div>
        {/if}
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
    {#if soundpad && sounds}
      <SoundpadOverlay />
    {/if}
    {#if dictationOpen && dictation && !dictation.needsModel}
      <DictationOverlay />
    {/if}
  {/if}
</div>

<style>
  :global(html), :global(body) { background: var(--bg-raised); background-image: none; overflow: hidden; }
  .overlay { position: relative; width: 840px; min-height: 368px; padding: 28px 32px 24px; background: var(--bg-raised); }
  /* Lines 12 px inside the window edge, each running 8 px past the corners. */
  .frame { position: absolute; inset: 12px; pointer-events: none; }
  .frame i { position: absolute; background: var(--line-strong); }
  .frame .t, .frame .b { left: -8px; right: -8px; height: 1px; }
  .frame .l, .frame .r { top: -8px; bottom: -8px; width: 1px; }
  .frame .t { top: 0; }
  .frame .b { bottom: 0; }
  .frame .l { left: 0; }
  .frame .r { right: 0; }
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
  .toggle { position: relative; overflow: hidden; }
  .toggle .progress { position: absolute; left: 0; bottom: 0; height: 2px; background: var(--accent-ink); }
  .sounds { display: flex; align-items: center; gap: 10px; min-width: 0; padding: 4px 0 0 2px; }
  .sounds-toggle.open { color: var(--accent-ink); border-color: var(--accent); }
  .sounds-label { flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 6px; font-size: 12px; color: var(--muted); }
  .sounds-label b { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-weight: 400; color: var(--accent-ink); }
  .sounds-label small { font-family: var(--font-mono); font-size: 10px; color: var(--faint); }
  .preset { margin-top: auto; padding-top: 8px; }
  .preset .hud-label { font-size: 9px; }
  .preset-nav { display: flex; gap: 8px; align-items: center; margin-top: 6px; }
  .preset-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: center; font-size: 12px; }
  .preset-name small { display: block; font-size: 9px; color: var(--app-amber-ink); }
  .status { flex: 1; align-self: center; padding: 24px; font-size: 13px; color: var(--muted); }
  /* Sketch: the app's own look — sharp corners, HUD labels, accent marks instead of fills. */
  .sketch .round, .sketch .toggle, .sketch kbd { border-radius: 0; }
  .sketch .toggle { position: relative; }
  .sketch .toggle small { font-family: var(--font-mono); letter-spacing: 0.1em; text-transform: uppercase; }
  .sketch .toggle.active::before, .sketch .toggle.alert::before { content: ''; position: absolute; left: -1px; top: -1px; bottom: -1px; width: 2px; background: var(--accent); }
  .sketch .toggle.alert::before { background: var(--danger); }
  .sketch .toggle.alert { border-color: var(--line); }
  .sketch .strip-name { font-family: var(--font-mono); font-size: 11px; font-weight: 500; letter-spacing: 0.15em; text-transform: uppercase; color: var(--muted); }
  @media (prefers-reduced-motion: reduce) { .round { transition: none; } }
</style>
