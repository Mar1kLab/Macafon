<script lang="ts">
  // First-run flow (PRD §9.2): welcome → pick the mic by voice → calibrate → result with A/B →
  // use it in apps (system default) → background start. Windows adds "Install VB-CABLE" after
  // welcome (WIN-MIC-002). Every step can be skipped; Settings runs it again. Nothing changes
  // on the system without an explicit switch (AS-001).
  import { onDestroy, tick, untrack } from 'svelte';
  import Logo from '../components/Logo.svelte';
  import Ticks from '../components/Ticks.svelte';
  import Toggle from '../components/Toggle.svelte';
  import DeviceLevel from '../mic/DeviceLevel.svelte';
  import CalibrationProgress from '../mic/CalibrationProgress.svelte';
  import CalibrationResult from '../mic/CalibrationResult.svelte';
  import TestPanel from '../mic/TestPanel.svelte';
  import { deviceIcon } from '../mic/devices';
  import { daemon } from '../state';
  import { deviceMeterBus } from '../state/meters';
  import { t, type MessageKey } from '../i18n';
  import { openVbCablePage } from '../ipc/tauri';

  let { onfinish }: { onfinish: (fineTune: boolean) => void } = $props();

  const ALL_STEPS = ['welcome', 'cable', 'mic', 'calibrate', 'result', 'use', 'mixer', 'background'] as const;
  type Step = (typeof ALL_STEPS)[number];
  /** The VB-CABLE step survives the reboot its installer asks for (WIN-MIC-002). */
  const RESUME_KEY = 'macafon.onboarding.resume';
  const CABLE_STEPS = ['unzip', 'run', 'install', 'reboot'] as const;
  /** A level above this counts as speech when picking the microphone by voice. */
  const SPEECH_DBFS = -45;
  /** Half a second of speech picks a microphone. */
  const PICK_TICKS = 5;
  const APPS = ['discord', 'obs', 'zoom'] as const;
  const PREVIEWS: Record<string, MessageKey> = {
    main: 'mixer.previews.main',
    voice: 'mixer.previews.voice',
    music: 'mixer.previews.music',
  };

  let step = $state<Step>(localStorage.getItem(RESUME_KEY) === 'cable' ? 'cable' : 'welcome');
  let heading: HTMLHeadingElement | undefined = $state();
  const STEPS = $derived(ALL_STEPS.filter((item) => item !== 'cable' || daemon.windows));
  const index = $derived(Math.max(0, STEPS.indexOf(step)));
  const mic = $derived(daemon.mic);
  const cableReady = $derived(mic?.virtual_mic_ready ?? false);

  $effect(() => {
    if (step === 'cable' && !cableReady) localStorage.setItem(RESUME_KEY, 'cable');
    else localStorage.removeItem(RESUME_KEY);
  });

  async function openDownloadPage() {
    try {
      await openVbCablePage();
    } catch (error) {
      daemon.fail(error);
    }
  }

  // --- Pick your mic ------------------------------------------------------------------------

  let picked = $state<string | null>(null);
  let manual = $state(false);
  let byVoice = $state(false);
  const speech = new Map<string, number>();

  function choose(name: string) {
    picked = name;
    manual = true;
    byVoice = false;
  }

  /** Until the user picks by hand, the microphone that hears speech the longest wins. */
  function listen() {
    let best: string | null = null;
    let bestScore = 0;
    for (const device of daemon.devices) {
      const peak = deviceMeterBus.peaks.get(device.name) ?? -120;
      const score = peak > SPEECH_DBFS ? (speech.get(device.name) ?? 0) + 1 : (speech.get(device.name) ?? 0) * 0.97;
      speech.set(device.name, score);
      if (score > bestScore) {
        best = device.name;
        bestScore = score;
      }
    }
    if (!manual && best && bestScore >= PICK_TICKS && best !== picked) {
      picked = best;
      byVoice = true;
    }
  }

  $effect(() => {
    if (step !== 'mic') return;
    untrack(() => (picked ??= mic?.device ?? daemon.devices[0]?.name ?? null));
    const unwatch = daemon.watchDevices();
    // Plugged-in microphones arrive with `devices.changed` (MIC-002).
    const timer = setInterval(() => {
      listen();
      untrack(() => (picked ??= daemon.devices[0]?.name ?? null));
    }, 100);
    return () => {
      clearInterval(timer);
      unwatch();
    };
  });

  // --- Calibrate ----------------------------------------------------------------------------

  let calibrationStarted = $state(false);
  const cal = $derived(daemon.calibration);
  const calibrated = $derived(calibrationStarted && cal.state === 'done');

  async function calibrate() {
    calibrationStarted = true;
    const result = await daemon.run(daemon.client.call('calibration.start'));
    if (result) daemon.calibration = result;
    else calibrationStarted = false;
  }

  $effect(() => {
    if (step === 'calibrate' && calibrated) void go('result');
  });

  function cancelCalibration() {
    if (daemon.calibration.state === 'measuring') void daemon.run(daemon.client.call('calibration.cancel'));
  }

  onDestroy(cancelCalibration);

  // --- Use it / background ------------------------------------------------------------------

  const systemDefault = $derived(mic?.is_default_source ?? false);
  let defaultBusy = $state(false);
  let startWithSystem = $state(true);
  /** "Fine-tune" still shows "Use it" and "Background", then opens the Microphone screen. */
  let fineTune = $state(false);

  async function setSystemDefault(on: boolean) {
    defaultBusy = true;
    const request = on
      ? daemon.client.call('mic.set_default_source', { confirmed: true })
      : daemon.client.call('mic.restore_default_source');
    await daemon.run(request);
    defaultBusy = false;
  }

  onDestroy(() => localStorage.removeItem(RESUME_KEY));

  async function done() {
    const autostart = daemon.autostart;
    if (autostart && autostart.enabled !== startWithSystem) {
      const state = await daemon.run(daemon.client.call('autostart.set', { enabled: startWithSystem }));
      if (state) daemon.autostart = state;
    }
    onfinish(fineTune);
  }

  // --- Navigation ---------------------------------------------------------------------------

  async function go(next: Step) {
    if (step === 'calibrate' && next !== 'result') cancelCalibration();
    step = next;
    await tick();
    heading?.focus();
  }

  async function forward(apply = true) {
    if (apply && step === 'mic' && picked && picked !== mic?.device) {
      await daemon.setMic(daemon.client.call('mic.set_device', { device: picked }));
    }
    // Without a calibration there is no result to show.
    if (step === 'calibrate' && !calibrated) return go('use');
    const next = STEPS[index + 1];
    if (next) await go(next);
  }

  function back() {
    const previous = step === 'use' && !calibrated ? 'calibrate' : STEPS[index - 1];
    if (previous) void go(previous);
  }

  const titles: Record<Step, [MessageKey, MessageKey]> = {
    welcome: ['onboarding.welcomeTitle', 'onboarding.welcomeAccent'],
    cable: ['onboarding.cableTitle', 'onboarding.cableAccent'],
    mic: ['onboarding.micTitle', 'onboarding.micAccent'],
    calibrate: ['onboarding.calibrateTitle', 'onboarding.calibrateAccent'],
    result: ['onboarding.resultTitle', 'onboarding.resultAccent'],
    use: ['onboarding.useTitle', 'onboarding.useAccent'],
    mixer: ['mixer.offTitle', 'mixer.offAccent'],
    background: ['onboarding.backgroundTitle', 'onboarding.backgroundAccent'],
  };
</script>

<div class="onboarding" role="dialog" aria-modal="true" aria-label={t('onboarding.label')}>
  <header class="top">
    <span class="brand"><Logo label={t('app.name')} />{t('app.name')}</span>
    <ol class="rail" aria-label={t('onboarding.step', { n: index + 1, total: STEPS.length })}>
      {#each STEPS as item, i (item)}
        <li class:current={i === index} class:passed={i < index} aria-current={i === index ? 'step' : undefined}>
          <span class="n">{String(i + 1).padStart(2, '0')}</span>
          <span class="hud-label">{t(`onboarding.steps.${item}`)}</span>
        </li>
      {/each}
    </ol>
    <button class="btn ghost" onclick={() => onfinish(false)}>{t('onboarding.skipAll')}</button>
  </header>

  <div class="body">
    {#key step}
      <section class="panel" class:wide={step === 'result'}>
        <Ticks />
        <span class="hud-label">{String(index + 1).padStart(2, '0')} / {String(STEPS.length).padStart(2, '0')}</span>
        <h1 bind:this={heading} tabindex="-1">{t(titles[step][0])} <em>{t(titles[step][1])}</em>.</h1>

        {#if step === 'welcome'}
          <p class="lead">{t('onboarding.welcomeBody')}</p>
        {:else if step === 'cable'}
          {#if cableReady}
            <p class="lead">{t('onboarding.cableReady')}</p>
          {:else}
            <p class="lead">{t('onboarding.cableBody')}</p>
            <ol class="howto">
              {#each CABLE_STEPS as item, i (item)}
                <li><span class="n">{String(i + 1).padStart(2, '0')}</span>{t(`onboarding.cableSteps.${item}`)}</li>
              {/each}
            </ol>
            <p class="note">{t('onboarding.cableDonation')}</p>
            <p class="note waiting" role="status">{t('onboarding.cableWaiting')}</p>
          {/if}
        {:else if step === 'mic'}
          <p class="lead">{t('onboarding.micBody')}</p>
          {#if daemon.devices.length === 0}
            <p class="note">{t('onboarding.micEmpty')}</p>
          {:else}
            <div class="devices" role="radiogroup" aria-label={t('mic.device')}>
              {#each daemon.devices as device (device.name)}
                {@const Icon = deviceIcon(device)}
                <button
                  class="device"
                  role="radio"
                  aria-checked={picked === device.name}
                  class:selected={picked === device.name}
                  onclick={() => choose(device.name)}
                >
                  <Icon size={16} strokeWidth={1.5} />
                  <span class="name">{device.description}</span>
                  {#if picked === device.name && byVoice}<span class="badge">{t('onboarding.micAuto')}</span>{/if}
                  <DeviceLevel name={device.name} segments={12} />
                </button>
              {/each}
            </div>
          {/if}
        {:else if step === 'calibrate'}
          {#if calibrationStarted && cal.state === 'measuring'}
            <CalibrationProgress state={cal} />
          {:else if calibrationStarted && cal.state === 'failed'}
            <p class="failed">{t(`calibration.failed.${cal.reason}`)}</p>
          {:else}
            <p class="lead">{t('calibration.intro')}</p>
            <p class="note">{t('mic.doubleProcessing')}</p>
          {/if}
        {:else if step === 'result'}
          {#if cal.state === 'done'}
            <CalibrationResult report={cal.report} />
            <p class="note">{t('onboarding.resultListenHint')}</p>
            <div class="transport"><TestPanel /></div>
          {:else}
            <p class="lead">{t('onboarding.resultSkipped')}</p>
          {/if}
        {:else if step === 'use'}
          <p class="lead">{daemon.windows ? t('onboarding.useBodyWindows') : t('onboarding.useBody')}</p>
          <div class="row">
            <div>
              <p class="name">{t('onboarding.makeDefault')}</p>
              <p class="note">{t('onboarding.makeDefaultHint')}</p>
            </div>
            <Toggle
              checked={systemDefault}
              disabled={defaultBusy || daemon.cableMissing}
              label={t('onboarding.makeDefault')}
              onchange={setSystemDefault}
            />
          </div>
          <ul class="apps">
            {#each APPS as app (app)}
              <li>
                <span class="hud-label">{t(`onboarding.apps.${app}`)}</span>
                <p>{t(`onboarding.apps.${app}How`)}</p>
              </li>
            {/each}
          </ul>
        {:else if step === 'mixer'}
          <p class="lead">{t('mixer.offBody')}</p>
          <ul class="apps channels">
            {#each daemon.mixer?.channels ?? [] as channel (channel.id)}
              <li style:--c="var(--app-{channel.color})">
                <span class="hud-label">{channel.name}</span>
                {#if PREVIEWS[channel.id]}<p>{t(PREVIEWS[channel.id]!)}</p>{/if}
              </li>
            {/each}
          </ul>
          <p class="note">{daemon.windows ? t('mixer.offNoteWindows') : t('mixer.offNote')}</p>
        {:else}
          <p class="lead">{t('onboarding.backgroundBody')}</p>
          <div class="row">
            <div>
              <p class="name">{t('settings.startWithSystem')}</p>
              <p class="note">{t('onboarding.startWithSystemHint')}</p>
            </div>
            <Toggle checked={startWithSystem} disabled={!daemon.autostart} label={t('settings.startWithSystem')} onchange={(on) => (startWithSystem = on)} />
          </div>
        {/if}
      </section>
    {/key}
  </div>

  <footer class="actions">
    {#if index > 0}
      <button class="btn ghost" onclick={back}>{t('onboarding.back')}</button>
    {/if}
    <span class="spacer"></span>
    {#if step === 'welcome'}
      <button class="btn invert" onclick={() => forward()}>{t('onboarding.getStarted')}</button>
    {:else if step === 'cable' && !cableReady}
      <button class="btn" onclick={() => forward()}>{t('onboarding.skip')}</button>
      <button class="btn invert" onclick={openDownloadPage}>{t('onboarding.cableOpen')}</button>
    {:else if step === 'calibrate'}
      {#if calibrationStarted && cal.state === 'measuring'}
        <button class="btn" onclick={cancelCalibration}>{t('calibration.cancel')}</button>
      {:else}
        <button class="btn" onclick={() => forward(false)}>{t('onboarding.skip')}</button>
        <button class="btn invert" onclick={calibrate}>
          {cal.state === 'failed' && calibrationStarted ? t('calibration.retry') : t('calibration.start')}
        </button>
      {/if}
    {:else if step === 'result'}
      {#if cal.state === 'done'}
        <button
          class="btn"
          onclick={() => {
            fineTune = true;
            void go('use');
          }}>{t('calibration.fineTune')}</button
        >
        <button class="btn" onclick={() => daemon.recordRequest++}>{t('calibration.listen')}</button>
      {/if}
      <button class="btn invert" onclick={() => forward()}>{t('onboarding.next')}</button>
    {:else if step === 'mixer'}
      {#if daemon.mixer?.enabled}
        <button class="btn invert" onclick={() => forward()}>{t('onboarding.next')}</button>
      {:else}
        <button class="btn" onclick={() => forward()}>{t('onboarding.skip')}</button>
        <button
          class="btn invert"
          onclick={async () => {
            await daemon.setMixer(daemon.client.call('mixer.set_enabled', { enabled: true }));
            await forward();
          }}>{t('mixer.enable')}</button
        >
      {/if}
    {:else if step === 'background'}
      <button class="btn invert" onclick={done}>{t('onboarding.done')}</button>
    {:else}
      {#if step === 'mic'}
        <button class="btn" onclick={() => forward(false)}>{t('onboarding.skip')}</button>
      {/if}
      <button class="btn invert" onclick={() => forward()} disabled={step === 'mic' && !picked}>{t('onboarding.next')}</button>
    {/if}
  </footer>
</div>

<style>
  .onboarding {
    display: grid;
    grid-template-rows: auto 1fr auto;
    grid-template-columns: minmax(0, 1fr);
    height: 100%;
    min-width: 0;
    min-height: 0;
  }

  .top {
    display: flex;
    align-items: center;
    gap: var(--space-6);
    height: 59px;
    padding: 0 var(--space-6);
    border-bottom: 1px solid var(--line);
    container: onboarding-top / inline-size;
  }


  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
  }

  .brand :global(svg) {
    width: 28px;
    height: 28px;
  }

  .rail {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: center;
    gap: var(--space-1);
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .rail li {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: 6px 10px;
    border-bottom: 2px solid var(--line);
    color: var(--faint);
  }

  .rail .hud-label {
    color: inherit;
  }

  /* A narrow window keeps the step numbers and only the current step's name, so Skip stays
     on screen (960 px minimum). */
  @container onboarding-top (max-width: 1120px) {
    .rail li:not(.current) .hud-label {
      display: none;
    }
  }

  .rail .n {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .rail .passed {
    color: var(--muted);
    border-bottom-color: var(--line-strong);
  }

  .rail .current {
    color: var(--ink);
    border-bottom-color: var(--accent);
  }

  .body {
    overflow-y: auto;
    display: flex;
    justify-content: center;
    align-items: safe center;
    padding: 24px;
  }

  .panel {
    position: relative;
    display: grid;
    gap: var(--space-4);
    width: 640px;
    padding: 32px;
    border: 1px solid var(--line);
    background: var(--bg-raised);
    animation: appear var(--dur-card) var(--ease-out);
  }

  .panel.wide {
    width: 880px;
  }

  h1 {
    font-family: var(--font-display);
    font-size: var(--text-display);
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1.1;
    color: var(--ink-strong);
    outline: none;
  }

  em {
    font-family: var(--font-serif);
    font-weight: 400;
    letter-spacing: 0;
  }

  .lead {
    font-size: var(--text-md);
    color: var(--ink);
  }

  .note {
    font-size: var(--text-md);
    color: var(--muted);
  }

  .failed {
    color: var(--danger);
  }

  .howto {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    border: 1px solid var(--line);
  }

  .howto li {
    display: flex;
    gap: var(--space-3);
    padding: 10px 16px;
    font-size: var(--text-md);
    color: var(--ink);
  }

  .howto li + li {
    border-top: 1px solid var(--line);
  }

  .howto .n {
    font-family: var(--font-mono);
    color: var(--accent);
  }

  .waiting {
    color: var(--app-amber-ink);
  }

  .devices {
    display: grid;
    border: 1px solid var(--line);
  }

  .device {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 48px;
    padding: 0 16px;
    text-align: left;
    color: var(--muted);
    border-left: 2px solid transparent;
    transition:
      background var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  .device + .device {
    border-top: 1px solid var(--line);
  }

  .device:hover {
    background: var(--bg-sunken);
    color: var(--ink);
  }

  .device.selected {
    color: var(--ink-strong);
    background: var(--bg-sunken);
    border-left-color: var(--accent);
  }

  .device .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .transport {
    border: 1px solid var(--line);
  }

  .transport :global(.test) {
    border-top: 0;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-6);
    padding: 14px 16px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .row .name {
    color: var(--ink);
  }

  .apps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-3);
  }

  .apps li {
    display: grid;
    align-content: start;
    gap: var(--space-2);
    padding: 12px;
    border: 1px solid var(--line);
  }

  .apps .hud-label {
    color: var(--ink);
  }

  .channels li {
    border-top: 3px solid var(--c);
  }

  .apps p {
    font-size: var(--text-sm);
    color: var(--muted);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 16px 24px;
    border-top: 1px solid var(--line);
  }

  .spacer {
    flex: 1;
  }

  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .panel {
      animation: none;
    }
  }
</style>
