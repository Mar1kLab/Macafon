<script lang="ts">
  import HotkeyHint from '../lib/settings/HotkeyHint.svelte';
  // 01 / MICROPHONE (PRD §9.3): device, preset, calibration, bypass and mute in the header;
  // IN/OUT meters; the eight module cards in signal order; the TEST panel pinned below.
  import { MicOff, Mic, Redo2, Undo2 } from '@lucide/svelte';
  import DevicePicker from '../lib/mic/DevicePicker.svelte';
  import PresetPicker from '../lib/mic/PresetPicker.svelte';
  import LevelMeter from '../lib/components/LevelMeter.svelte';
  import GainCard from '../lib/mic/cards/GainCard.svelte';
  import HighPassCard from '../lib/mic/cards/HighPassCard.svelte';
  import NoiseCard from '../lib/mic/cards/NoiseCard.svelte';
  import ExpanderCard from '../lib/mic/cards/ExpanderCard.svelte';
  import EqCard from '../lib/mic/cards/EqCard.svelte';
  import CompressorCard from '../lib/mic/cards/CompressorCard.svelte';
  import DeEsserCard from '../lib/mic/cards/DeEsserCard.svelte';
  import LimiterCard from '../lib/mic/cards/LimiterCard.svelte';
  import TestPanel from '../lib/mic/TestPanel.svelte';
  import CalibrationDialog from '../lib/mic/CalibrationDialog.svelte';
  import GettingStarted from '../lib/mic/GettingStarted.svelte';
  import { daemon } from '../lib/state';
  import { SAMPLE_RATE_HZ } from '../lib/ipc/dsp';
  import { samplesToMs } from '../lib/dsp/format';
  import { t } from '../lib/i18n';
  import { openVbCablePage } from '../lib/ipc/tauri';

  const mic = $derived(daemon.mic);
  let calibrating = $state(false);
  let eqExpanded = $state(false);
  let inputClipped = $state(false);

  /** Calibration done → record a clip right away so before/after can be compared. */
  function listen() {
    calibrating = false;
    daemon.recordRequest++;
  }

  const setBypass = (bypass: boolean) => daemon.setMic(daemon.client.call('mic.set_bypass', { bypass }));

  function onkeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (target.closest('input, textarea')) return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') {
      event.preventDefault();
      if (event.shiftKey) daemon.redo();
      else daemon.undo();
    }
  }
</script>

<svelte:window {onkeydown} />

{#if mic}
  <div class="screen">
    <header class="top">
      <h1 class="hud-label">{t('mic.section')}</h1>
      <DevicePicker {mic} />
      <PresetPicker {mic} />
      <span class="spacer"></span>
      <div class="history">
        <button class="btn icon ghost" aria-label={t('mic.undo')} title="{t('mic.undo')} (Ctrl+Z)" disabled={!daemon.canUndo} onclick={() => daemon.undo()}>
          <Undo2 size={15} strokeWidth={1.5} />
        </button>
        <button class="btn icon ghost" aria-label={t('mic.redo')} title="{t('mic.redo')} (Ctrl+Shift+Z)" disabled={!daemon.canRedo} onclick={() => daemon.redo()}>
          <Redo2 size={15} strokeWidth={1.5} />
        </button>
      </div>
      <!-- Until the first calibration it is the most useful next step: make it the CTA. -->
      <button class="btn" class:invert={mic.noise_floor_dbfs === null} title={t('mic.calibrateHint')} onclick={() => (calibrating = true)}>
        {t('mic.calibrate')}
      </button>
      <button
        class="btn"
        class:on={mic.settings.bypass}
        aria-pressed={mic.settings.bypass}
        title={t('mic.bypassHint')}
        onclick={() => setBypass(!mic.settings.bypass)}
      >
        {t('mic.bypass')}<HotkeyHint actions={['bypass_toggle']} />
      </button>
      <button
        class="btn danger"
        class:on={mic.settings.muted}
        aria-pressed={mic.settings.muted}
        title={t('mic.muteHint')}
        onclick={() => daemon.setMic(daemon.client.call('mic.set_muted', { muted: !mic.settings.muted }))}
      >
        {#if mic.settings.muted}<MicOff size={13} strokeWidth={1.5} />{t('mic.muted')}{:else}<Mic size={13} strokeWidth={1.5} />{t('mic.mute')}{/if}
        <HotkeyHint actions={['mic_mute_toggle', 'push_to_talk', 'push_to_mute']} />
      </button>
    </header>

    <div class="meters">
      <LevelMeter label={t('mic.input')} hint={t('mic.inputHint')} peakKey="input_peak_dbfs" rmsKey="input_rms_dbfs" clipKey="raw_peak_dbfs" onclip={(c) => (inputClipped = c)} />
      <span class="latency" title={t('mic.latencyHint')}>
        <span class="hud-label">{t('mic.latency')}</span>
        <span class="value">{samplesToMs(mic.latency_samples, SAMPLE_RATE_HZ).toFixed(1)} ms</span>
      </span>
      <LevelMeter label={t('mic.output')} hint={t('mic.outputHint')} peakKey="output_peak_dbfs" rmsKey="output_rms_dbfs" color="teal" />
    </div>
    {#if daemon.cableMissing}
      <p class="banner danger" role="status">
        <strong>{t('mic.cableMissing')}.</strong>
        {t('mic.cableMissingBody')}
        <button class="btn small" onclick={() => void openVbCablePage().catch((error) => daemon.fail(error))}>{t('mic.cableInstall')}</button>
      </p>
    {/if}
    {#if !mic.device}
      <p class="banner" role="status">{t('mic.bannerNoDevice')}</p>
    {:else if !mic.input_connected}
      <p class="banner danger" role="status">{t('mic.bannerDisconnected')}</p>
    {/if}
    {#if mic.settings.bypass}
      <p class="banner" role="status">
        {t('mic.bannerBypass')}
        <button class="btn small" onclick={() => setBypass(false)}>{t('mic.bannerBypassAction')}</button>
      </p>
    {/if}
    {#if inputClipped}
      <p class="banner danger" role="alert">{t('mic.clipHint')}</p>
    {/if}

    <div class="scroll">
      <GettingStarted oncalibrate={() => (calibrating = true)} calibrated={mic.noise_floor_dbfs !== null} />
      <div class="chain" class:bypassed={mic.settings.bypass} class:eq-expanded={eqExpanded}>
        <GainCard settings={mic.settings} oncalibrate={() => (calibrating = true)} />
        <HighPassCard settings={mic.settings} />
        <NoiseCard settings={mic.settings} />
        <ExpanderCard {mic} />
        <div class="eq-slot"><EqCard settings={mic.settings} expanded={eqExpanded} onexpand={() => (eqExpanded = !eqExpanded)} /></div>
        <CompressorCard settings={mic.settings} />
        <DeEsserCard settings={mic.settings} />
        <LimiterCard settings={mic.settings} />
      </div>
    </div>

    <TestPanel />
  </div>
  <CalibrationDialog open={calibrating} onclose={() => (calibrating = false)} onlisten={listen} />
{:else}
  <div class="loading" role="status">
    <span class="hud-label">{t('daemon.loading')}</span>
  </div>
{/if}

<style>
  .screen {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }

  .top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 10px;
    padding: 14px 24px;
    border-bottom: 1px solid var(--line);
  }

  h1 {
    color: var(--ink);
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }

  .history {
    display: flex;
  }

  .meters {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    gap: 32px;
    align-items: center;
    padding: 12px 24px;
    border-bottom: 1px solid var(--line);
  }

  .latency {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 24px;
    font-size: 13px;
    color: var(--ink);
    background: var(--bg-raised);
    border-bottom: 1px solid var(--line);
    border-left: 2px solid var(--accent);
  }

  .banner.danger {
    color: var(--danger);
    border-left-color: var(--danger);
  }

  .loading {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 16px 24px 24px;
  }

  .chain {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 16px;
    align-items: start;
    transition: opacity var(--dur) var(--ease);
  }

  /* EQ takes two columns; expanded, the whole row (§9.3). */
  .eq-slot {
    grid-column: span 2;
    display: grid;
  }

  .eq-expanded .eq-slot {
    grid-column: 1 / -1;
  }

  .bypassed {
    opacity: 0.55;
  }

  /* Thin "wires" between neighbouring cards in signal order. */
  .chain > :global(*:not(:nth-child(3n + 1))) {
    position: relative;
  }

  .chain > :global(:nth-child(2))::after,
  .chain > :global(:nth-child(3))::after,
  .chain > :global(:nth-child(5))::after,
  .chain > :global(:nth-child(7))::after,
  .chain > :global(:nth-child(8))::after {
    content: '';
    position: absolute;
    left: -17px;
    top: 17px;
    width: 16px;
    height: 1px;
    background: var(--line-strong);
  }

  .eq-expanded > :global(:nth-child(5))::after {
    display: none;
  }
</style>
