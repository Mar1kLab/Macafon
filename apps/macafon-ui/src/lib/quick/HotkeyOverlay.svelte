<script lang="ts">
  import { AudioLines, AudioWaveform, ChevronDown, ChevronLeft, ChevronRight, Headphones, Mic, MicOff, SlidersHorizontal, Speech, Volume2, VolumeX, X } from '@lucide/svelte';
  import Logo from '../components/Logo.svelte';
  import LevelMeter from '../components/LevelMeter.svelte';
  import VerticalSlider from './VerticalSlider.svelte';
  import SoundpadOverlay from './SoundpadOverlay.svelte';
  import DictationOverlay from './DictationOverlay.svelte';
  import { DEFAULT_OVERLAY, reveal } from './overlay';
  import { overlayOrder, overlayRows, volumesFirst } from './layout';
  import type { OverlayItem } from '../ipc/protocol';
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
  /** What the overlay shows, where its side panels open and how it is placed (Settings). */
  const setup = $derived(daemon.prefs?.overlay ?? DEFAULT_OVERLAY);
  const expand = $derived(setup.expand);
  const Flag = $derived(expand === 'left' ? ChevronLeft : expand === 'down' ? ChevronDown : ChevronRight);
  const playing = $derived(playback && soundpad?.sounds.find((sound) => sound.id === playback.sound_id));
  const output = $derived(mixer?.outputs.find((item) => item.name === mixer.output) ?? null);
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  /** The parts in the order the layout editor saved, only those that show (§9.9). */
  const parts = $derived(
    overlayOrder(setup.order).filter((part: OverlayItem) => {
      switch (part) {
        case 'dictation':
          return !!dictation;
        case 'sounds':
          return !!soundpad;
        case 'output_device':
          return setup.output_device && !!mixer;
        default:
          return setup[part];
      }
    }),
  );
  const rows = $derived(overlayRows(parts));
  /** Where the Sounds column shows the player, if at all. */
  const dock = $derived(soundpad?.prefs.overlay_controls ?? 'hidden');
  const volumesLeft = $derived(volumesFirst(parts));

  /** SND-006, §9.9.2: one side panel at a time, the way the overlay was last closed, across
   * openings and UI restarts. */
  type Side = 'sounds' | 'dictation' | null;
  const PANEL_KEY = 'macafon.overlay.panel';
  function savedPanel(): Side {
    const saved = localStorage.getItem(PANEL_KEY);
    if (saved === 'sounds' || saved === 'dictation') return saved;
    // Before one key held both.
    if (localStorage.getItem('macafon.overlay.soundsOpen') === '1') return 'sounds';
    if (localStorage.getItem('macafon.overlay.dictationOpen') === '1') return 'dictation';
    return null;
  }
  let panel = $state<Side>(savedPanel());
  const side = $derived(panel === 'sounds' && soundpad ? 'sounds' : panel === 'dictation' && dictation && !dictation.needsModel ? 'dictation' : null);

  function toggle(next: Exclude<Side, null>) {
    panel = panel === next ? null : next;
    localStorage.setItem(PANEL_KEY, panel ?? '');
  }

  // The page stays loaded between openings: each one restores what the last left.
  $effect(() => {
    void opened;
    panel = savedPanel();
  });

  /** OUT-001: the next or previous output device, in the order the system lists them. */
  function cycleOutput(step: number) {
    if (!mixer || mixer.outputs.length === 0) return;
    const index = mixer.outputs.findIndex((item) => item.name === mixer.output);
    const next = mixer.outputs[(index + step + mixer.outputs.length) % mixer.outputs.length]!;
    void daemon.setMixer(daemon.client.call('mixer.set_output', { device: next.name }));
  }

  // The microphone level needs live meters, which this window has off unless it shows one.
  $effect(() => {
    if (setup.mic_level) return daemon.watchMeters();
  });
</script>

{#snippet flag(open: boolean, label: string, onclick: () => void, disabled = false)}
  <button class="flag" class:open aria-expanded={open} aria-label={label} title={label} {disabled} {onclick}>
    <Flag size={13} strokeWidth={1.5} />
  </button>
{/snippet}

{#snippet control(part: OverlayItem)}
  {#if part === 'mic_switch'}
    <button class="switch" class:on={!muted} class:alert={muted} role="switch" aria-checked={!muted} title={muted ? t('quick.muted') : t('quick.on')} onclick={() => daemon.setMic(daemon.client.call('mic.set_muted', { muted: !muted }))}>
      {#if muted}<MicOff size={16} strokeWidth={1.5} />{:else}<Mic size={16} strokeWidth={1.5} />{/if}<span>{t('quick.microphone')}</span>
    </button>
  {:else if part === 'processing_switch'}
    <button class="switch" class:on={!bypass} role="switch" aria-checked={!bypass} title={bypass ? t('quick.bypassed') : t('quick.on')} onclick={() => daemon.setMic(daemon.client.call('mic.set_bypass', { bypass: !bypass }))}>
      <AudioLines size={16} strokeWidth={1.5} /><span>{t('quick.processing')}</span>
    </button>
  {:else if part === 'mixer_switch'}
    <button class="switch" class:on={mixer?.enabled} role="switch" aria-checked={mixer?.enabled ?? false} disabled={!mixer} onclick={() => mixer && daemon.setMixer(daemon.client.call('mixer.set_enabled', { enabled: !mixer.enabled }))}>
      <SlidersHorizontal size={16} strokeWidth={1.5} /><span>{t('quick.mixer')}</span>
    </button>
  {:else if part === 'dictation' && dictation}
    <div class="split {expand}" class:open={side === 'dictation'}>
      <button class="switch" class:on={dictation.on} role="switch" aria-checked={dictation.on} disabled={dictation.needsModel} title={`${t('quick.dictationHint')} · ${dictation.state}`} onclick={() => daemon.setDictationPrefs({ enabled: !dictation.on })}>
        <Speech size={16} strokeWidth={1.5} /><span>{t('quick.dictation')}</span>
        {#if dictation.progress !== null}<i class="progress" style:width="{dictation.progress}%"></i>{/if}
      </button>
      {@render flag(side === 'dictation', side === 'dictation' ? t('quick.dictationHide') : t('quick.dictationShow'), () => toggle('dictation'), dictation.needsModel)}
    </div>
  {:else if part === 'sounds' && soundpad}
    <div class="split {expand}" class:open={side === 'sounds'}>
      <button class="switch" class:on={!!playback} aria-expanded={side === 'sounds'} aria-controls="overlay-sounds" title={playing ? playing.name : side === 'sounds' ? t('quick.soundpadHide') : t('quick.soundpadShowHint')} onclick={() => toggle('sounds')}>
        <AudioWaveform size={16} strokeWidth={1.5} /><span>{playing ? playing.name : t('quick.soundpadShow')}</span>
      </button>
      {@render flag(side === 'sounds', side === 'sounds' ? t('quick.soundpadHide') : t('quick.soundpadShowHint'), () => toggle('sounds'))}
    </div>
  {:else if part === 'output_device' && mixer}
    <div class="picker">
      <span class="hud-label">{t('quick.output')}</span>
      <div class="picker-nav">
        <button class="round" aria-label={t('quick.previousOutput')} title={t('quick.previousOutput')} disabled={mixer.outputs.length < 2} onclick={() => cycleOutput(-1)}><ChevronLeft size={16} strokeWidth={1.5} /></button>
        <span class="picker-name" title={output?.description ?? t('mixer.noOutput')} aria-live="polite"><Headphones size={13} strokeWidth={1.5} />{output?.description ?? t('mixer.noOutput')}</span>
        <button class="round" aria-label={t('quick.nextOutput')} title={t('quick.nextOutput')} disabled={mixer.outputs.length < 2} onclick={() => cycleOutput(1)}><ChevronRight size={16} strokeWidth={1.5} /></button>
      </div>
    </div>
  {:else if part === 'mic_level'}
    <div class="level" class:muted>
      <LevelMeter label={t('quick.micLevel')} peakKey="output_peak_dbfs" rmsKey="output_rms_dbfs" color="lime" />
    </div>
  {:else if part === 'preset' && mic}
    <div class="picker">
      <span class="hud-label">{t('quick.preset')}</span>
      <div class="picker-nav">
        <button class="round" aria-label={t('quick.previousPreset')} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: -1 }))}><ChevronLeft size={16} strokeWidth={1.5} /></button>
        <span class="picker-name" title={preset} aria-live="polite">{preset}{#if mic.preset_modified}<small>{t('quick.modified')}</small>{/if}</span>
        <button class="round" aria-label={t('quick.nextPreset')} onclick={() => daemon.setMic(daemon.client.call('mic.cycle_preset', { step: 1 }))}><ChevronRight size={16} strokeWidth={1.5} /></button>
      </div>
    </div>
  {/if}
{/snippet}

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
    <div class="body" class:left={expand === 'left'} class:down={expand === 'down'}>
      <!-- Anchored, the window keeps this part in place when a side panel opens (Panel.svelte). -->
      <div class="main" class:compact={!setup.volumes} class:volumes-right={!volumesLeft} data-overlay-main data-overlay-centered={setup.placement === 'centered' ? '' : undefined}>
        {#if setup.volumes}
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
        {/if}

        <section class="controls" aria-label={t('quick.microphone')}>
          <!-- Small switches: on is filled, off is a plain outline. Dictation and Sounds carry a
               flag on the side their panel opens to. -->
          <!-- The parts in the order of the layout editor: neighbouring switches share rows, the
               rest take a row each, the last one sits at the bottom. -->
          {#each rows as row, index (row.kind === 'wide' ? row.part : `switches-${index}`)}
            {#if row.kind === 'switches'}
              <div class="switches">
                {#each row.parts as part (part)}{@render control(part)}{/each}
              </div>
            {:else}
              <div class="wide" class:last={index > 0 && index === rows.length - 1}>{@render control(row.part)}</div>
            {/if}
          {/each}
        </section>
      </div>

      {#if side}
        <!-- One column with its own fixed size: the window resizes once, then the content
             fades in; the player belongs to the Sounds column, above or under its list. -->
        <div class="side-reveal" transition:reveal={{ expand, duration: reduced ? 0 : 200 }}>
          <aside class="side" class:sounds={side === 'sounds'}>
            {#if side === 'sounds'}
              {#if dock === 'top'}<div class="dock top"><SoundpadOverlay part="player" wide={expand === 'down'} /></div>{/if}
              <SoundpadOverlay part="library" />
              {#if dock === 'bottom'}<div class="dock"><SoundpadOverlay part="player" wide={expand === 'down'} /></div>{/if}
            {:else}
              <DictationOverlay />
            {/if}
          </aside>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  :global(html), :global(body) { background: var(--bg-raised); background-image: none; overflow: hidden; }
  .overlay { position: relative; width: max-content; min-height: 368px; padding: 28px 32px 24px; background: var(--bg-raised); }
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
  .round:hover:not(:disabled) { border-color: var(--line-strong); color: var(--ink-strong); }
  .round:disabled { opacity: 0.4; }
  .close { background: var(--bg); }

  /* The side panel opens to the right, to the left (the main part stays first in the page) or
     under the main part. */
  .body { display: flex; padding-top: 16px; }
  .body.left { flex-direction: row-reverse; }
  .body.down { flex-direction: column; }
  /* Side content never stretches the main part; the player docks under Sounds. */
  .body { align-items: flex-start; }
  .body.down { align-items: stretch; }
  .main { flex: none; }
  /* As wide as the overlay was before side panels, a little wider: the strips share it. */
  .main { display: flex; gap: 24px; min-width: 816px; }
  .main.compact { min-width: 0; }
  .volumes { flex: 1; min-width: 0; display: flex; overflow-x: auto; padding: 0 2px 4px; }
  .strip { flex: 1 0 88px; min-width: 0; display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .master { flex-grow: 0; border-right: 1px solid var(--line); margin-right: 8px; padding-right: 8px; }
  .strip-name { font-family: var(--font-display); font-size: 13px; font-weight: 600; color: var(--ink-strong); max-width: 88px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .mute { color: var(--channel-ink); }
  .alert, .round.alert { color: var(--danger); border-color: var(--danger); }
  .controls { width: 240px; flex: none; padding-left: 24px; border-left: 1px solid var(--line); display: flex; flex-direction: column; gap: 10px; }
  .compact .controls { padding-left: 0; border-left: 0; }
  /* Volumes after the controls in the layout: on the right of them. */
  .main.volumes-right { flex-direction: row-reverse; }
  .volumes-right .controls { padding: 0 24px 0 0; border-left: 0; border-right: 1px solid var(--line); }
  .compact.volumes-right .controls { padding-right: 0; border-right: 0; }
  /* The last row sits at the bottom of the column, along the bottom of the strips. */
  .wide.last { margin-top: auto; padding-top: 4px; }

  .switches { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; }
  /* An odd last switch takes the whole row. */
  .switches > :last-child:nth-child(odd) { grid-column: span 2; }
  .switch { position: relative; overflow: hidden; min-width: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; height: 54px; padding: 4px; border: 1px solid var(--line); border-radius: 12px; background: transparent; color: var(--faint); font-size: 11px; transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease), color var(--dur) var(--ease); }
  .switch span { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .switch:hover:not(:disabled) { border-color: var(--line-strong); color: var(--ink); }
  .switch.on { background: var(--accent); border-color: var(--accent); color: var(--ink-strong); }
  .switch.alert { border-color: var(--danger); color: var(--danger); }
  .switch:disabled { opacity: 0.4; }
  .switch .progress { position: absolute; left: 0; bottom: 0; height: 2px; background: var(--accent-ink); }

  /* A switch with its flag: one control, the flag on the side the panel opens to. */
  .split { display: flex; min-width: 0; }
  .split.left { flex-direction: row-reverse; }
  .split.down { flex-direction: column; }
  .split .switch { flex: 1; }
  .split.right .switch { border-radius: 12px 0 0 12px; border-right-width: 0; }
  .split.left .switch { border-radius: 0 12px 12px 0; border-left-width: 0; }
  .split.down .switch { height: 40px; border-radius: 12px 12px 0 0; border-bottom-width: 0; }
  .flag { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 18px; border: 1px solid var(--line); background: var(--bg-sunken); color: var(--muted); transition: border-color var(--dur) var(--ease), color var(--dur) var(--ease), background var(--dur) var(--ease); }
  .split.right .flag { border-radius: 0 12px 12px 0; }
  .split.left .flag { border-radius: 12px 0 0 12px; }
  .split.down .flag { width: auto; height: 14px; border-radius: 0 0 12px 12px; }
  .flag :global(svg) { transition: transform var(--dur) var(--ease); }
  .flag:hover:not(:disabled) { border-color: var(--line-strong); color: var(--ink-strong); }
  .flag:disabled { opacity: 0.4; }
  .split.open .flag { border-color: var(--accent); background: var(--accent); color: var(--ink-strong); }
  .split.open .flag :global(svg) { transform: rotate(180deg); }
  .switch:focus-visible, .flag:focus-visible, .round:focus-visible { outline: 1px solid var(--ink); outline-offset: 2px; position: relative; z-index: 1; }

  .picker .hud-label { font-size: 9px; }
  .picker-nav { display: flex; gap: 8px; align-items: center; margin-top: 6px; }
  .picker-name { flex: 1; min-width: 0; display: flex; align-items: center; justify-content: center; gap: 6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: center; font-size: 12px; }
  .picker-name :global(svg) { flex: none; color: var(--muted); }
  .picker-name small { margin-left: 6px; font-size: 9px; color: var(--app-amber-ink); }
  .level :global(.meter) { grid-template-columns: auto 1fr auto 36px auto; gap: 8px; }
  .level :global(.hud-label) { font-size: 9px; }
  .level.muted { opacity: 0.5; }

  .side-reveal { flex: none; display: flex; min-width: 0; min-height: 0; }
  .body.left .side-reveal { justify-content: flex-end; }
  /* Fixed sizes: what loads inside (sounds, history) scrolls instead of resizing the window. */
  .side { width: 440px; height: 380px; flex: none; margin-left: 24px; padding-left: 24px; border-left: 1px solid var(--line); display: flex; flex-direction: column; min-height: 0; }
  .body.left .side { margin: 0 24px 0 0; padding: 0 24px 0 0; border-left: 0; border-right: 1px solid var(--line); }
  .body.down .side { width: auto; height: 300px; flex: 1; margin: 16px 0 0; padding: 16px 0 0; border-left: 0; border-top: 1px solid var(--line); }
  .dock { flex: none; margin-top: 12px; padding-top: 12px; border-top: 1px solid var(--line); }
  .dock.top { margin: 0 0 12px; padding: 0 0 12px; border-top: 0; border-bottom: 1px solid var(--line); }
  .status { flex: 1; align-self: center; padding: 24px; font-size: 13px; color: var(--muted); }
  /* Sketch: the app's own look — sharp corners, HUD labels, accent marks instead of fills. */
  .sketch .round, .sketch .switch, .sketch .flag, .sketch .split .switch, .sketch .split .flag, .sketch kbd { border-radius: 0; }
  .sketch .switch span { font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.1em; text-transform: uppercase; }
  .sketch .strip-name { font-family: var(--font-mono); font-size: 11px; font-weight: 500; letter-spacing: 0.15em; text-transform: uppercase; color: var(--muted); }
  @media (prefers-reduced-motion: reduce) { .round, .switch, .flag, .flag :global(svg) { transition: none; } }
</style>
