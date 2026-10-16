<script lang="ts">
  // 03 / SETTINGS (PRD §9.5) — what the daemon supports today: start with system (AS-001),
  // the setup guide (§9.2), fallback microphone (MIC-006), input channel (MIC-004), system
  // default microphone (MIC-012), virtual microphone format (MIC-011), quit (BG-005), about.
  // Mixer: known-app sorting (APP-006) and the output priority list (OUT-002).
  // Hotkeys (HK-002, HK-004). Doctor (§9.6).
  import Toggle from '../lib/components/Toggle.svelte';
  import Dropdown from '../lib/components/Dropdown.svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import { ArrowDown, ArrowUp, X } from '@lucide/svelte';
  import { daemon } from '../lib/state';
  import Doctor from '../lib/settings/Doctor.svelte';
  import Hotkeys from '../lib/settings/Hotkeys.svelte';
  import { t, type MessageKey } from '../lib/i18n';
  import type { AutostartMethod, CloseAction, InputChannel } from '../lib/ipc/protocol';
  import { QUANTUM_CHOICES, SAMPLE_RATE_HZ } from '../lib/ipc/dsp';

  const autostartMethodKey: Record<AutostartMethod, MessageKey> = {
    systemd: 'settings.methodSystemd',
    xdg_autostart: 'settings.methodXdg',
    windows_run: 'settings.methodWindowsRun',
  };

  let { onnavigate }: { onnavigate: (screen: 'microphone' | 'mixer') => void } = $props();
  const mic = $derived(daemon.mic);
  const mixer = $derived(daemon.mixer);
  const describe = (name: string) => mixer?.outputs.find((output) => output.name === name)?.description ?? name;
  const unlisted = $derived(
    (mixer?.outputs ?? [])
      .filter((output) => !mixer!.output_priority.includes(output.name))
      .map((output) => ({ value: output.name, label: output.description })),
  );

  function setPriority(devices: string[]) {
    void daemon.setMixer(daemon.client.call('mixer.set_output_priority', { devices }));
  }

  function shift(index: number, delta: number) {
    if (!mixer) return;
    const list = [...mixer.output_priority];
    const [item] = list.splice(index, 1);
    list.splice(index + delta, 0, item!);
    setPriority(list);
  }
  const autostart = $derived(daemon.autostart);
  const fallbackOptions = $derived([
    { value: null as string | null, label: t('settings.none') },
    ...daemon.devices
      .filter((device) => device.name !== mic?.device)
      .map((device) => ({ value: device.name as string | null, label: device.description })),
  ]);
  const CHANNELS: InputChannel[] = ['Auto', 'Left', 'Right', 'Mix'];
  const channelOptions = CHANNELS.map((value) => ({ value, label: t(`settings.channels.${value}`) }));
  const prefs = $derived(daemon.prefs);
  const quantumOptions = QUANTUM_CHOICES.map((frames) => ({
    value: frames,
    label: t('settings.quantumValue', { frames, ms: ((frames / SAMPLE_RATE_HZ) * 1000).toFixed(1) }),
  }));
  const closeOptions = (['tray', 'quit'] as CloseAction[]).map((value) => ({
    value,
    label: t(`settings.closeActions.${value}`),
  }));

  async function setPrefs(patch: { quantum_frames?: number; close_action?: CloseAction }) {
    const next = await daemon.run(
      daemon.client.call('prefs.set', { quantum_frames: patch.quantum_frames ?? null, close_action: patch.close_action ?? null }),
    );
    if (next) daemon.prefs = next;
  }
  let confirmQuit = $state(false);
  let confirmDefault = $state(false);
  const version = $derived(daemon.status.state === 'connected' ? daemon.status.version : '—');

  async function setAutostart(enabled: boolean) {
    const state = await daemon.run(daemon.client.call('autostart.set', { enabled }));
    if (state) daemon.autostart = state;
  }

  // MIC-012: the switch shows what PipeWire reports; turning it on asks first.
  async function makeDefault() {
    confirmDefault = false;
    await daemon.run(daemon.client.call('mic.set_default_source', { confirmed: true }));
  }

  async function setDefault(on: boolean) {
    if (on) confirmDefault = true;
    else await daemon.run(daemon.client.call('mic.restore_default_source'));
  }
</script>

<div class="screen">
  <header class="top">
    <h1 class="hud-label">03 / {t('settings.section')}</h1>
  </header>
  <div class="scroll">
    <section>
      <h2 class="hud-label">{t('settings.general')}</h2>
      <div class="row">
        <div>
          <p class="name">{t('settings.startWithSystem')}</p>
          {#if autostart}
            <p class="hint">
              {t('settings.startWithSystemHint', { method: t(autostartMethodKey[autostart.method]) })}
            </p>
          {/if}
        </div>
        <Toggle checked={autostart?.enabled ?? false} disabled={!autostart} label={t('settings.startWithSystem')} onchange={setAutostart} />
      </div>
      <div class="row">
        <div>
          <p class="name">{t('settings.onboarding')}</p>
          <p class="hint">{t('settings.onboardingHint')}</p>
        </div>
        <button class="btn" onclick={() => daemon.startOnboarding()}>{t('settings.onboardingRun')}</button>
      </div>
      {#if prefs}
        <div class="row">
          <div>
            <p class="name">{t('settings.closeAction')}</p>
            <p class="hint">{t('settings.closeActionHint')}</p>
          </div>
          <Dropdown
            label={t('settings.closeAction')}
            options={closeOptions}
            value={prefs.close_action}
            width="280px"
            onchange={(close_action) => setPrefs({ close_action })}
          />
        </div>
      {/if}
      <div class="row">
        <div>
          <p class="name">{t('settings.quit')}</p>
          <p class="hint">{daemon.windows ? t('settings.quitHintWindows') : t('settings.quitHint')}</p>
        </div>
        <button class="btn" onclick={() => (confirmQuit = true)}>{t('settings.quit')}</button>
      </div>
    </section>

    {#if mic}
      <section>
        <h2 class="hud-label">{t('settings.audio')}</h2>
        <div class="row">
          <div>
            <p class="name">{t('settings.fallback')}</p>
            <p class="hint">{t('settings.fallbackHint')}</p>
          </div>
          <Dropdown
            label={t('settings.fallback')}
            options={fallbackOptions}
            value={mic.fallback_device}
            width="280px"
            onchange={(device) => daemon.setMic(daemon.client.call('mic.set_fallback_device', { device }))}
          />
        </div>
        <div class="row">
          <div>
            <p class="name">{t('settings.inputChannel')}</p>
            <p class="hint">{t('settings.inputChannelHint')}</p>
          </div>
          <Dropdown
            label={t('settings.inputChannel')}
            options={channelOptions}
            value={mic.input_channel ?? 'Auto'}
            width="280px"
            onchange={(channel) => daemon.setMic(daemon.client.call('mic.set_input_channel', { channel }))}
          />
        </div>
        <div class="row">
          <div>
            <p class="name">{t('settings.systemDefault')}</p>
            <p class="hint">{t('settings.systemDefaultHint')}</p>
          </div>
          <Toggle checked={mic.is_default_source} disabled={daemon.cableMissing} label={t('settings.systemDefault')} onchange={setDefault} />
        </div>
        {#if prefs && (daemon.hello?.capabilities.graph_quantum ?? true)}
          <div class="row">
            <div>
              <p class="name">{t('settings.quantum')}</p>
              <p class="hint">{t('settings.quantumHint')}</p>
            </div>
            <Dropdown
              label={t('settings.quantum')}
              options={quantumOptions}
              value={prefs.quantum_frames}
              width="280px"
              onchange={(quantum_frames) => setPrefs({ quantum_frames })}
            />
          </div>
        {/if}
        <div class="row">
          <div>
            <p class="name">{t('settings.format')}</p>
            <p class="hint">{daemon.windows ? t('settings.formatHintWindows') : t('settings.formatHint')}</p>
          </div>
          <span class="badge">{mic.stereo || daemon.windows ? t('settings.stereo') : t('settings.mono')} · 48 kHz · float32</span>
        </div>
      </section>
    {/if}

    {#if mixer}
      <section>
        <h2 class="hud-label">{t('settings.mixer')}</h2>
        <div class="row">
          <div>
            <p class="name">{t('settings.autoClassify')}</p>
            <p class="hint">{t('settings.autoClassifyHint')}</p>
          </div>
          <Toggle
            checked={mixer.auto_classify}
            label={t('settings.autoClassify')}
            onchange={(enabled) => daemon.setMixer(daemon.client.call('mixer.set_auto_classify', { enabled }))}
          />
        </div>
        <div class="row column">
          <div>
            <p class="name">{t('settings.outputPriority')}</p>
            <p class="hint">{t('settings.outputPriorityHint')}</p>
          </div>
          <ol class="priority">
            {#each mixer.output_priority as name, index (name)}
              <li class:missing={!mixer.outputs.some((o) => o.name === name)}>
                <span class="n">{index + 1}</span>
                <span class="device">{describe(name)}</span>
                {#if !mixer.outputs.some((o) => o.name === name)}<span class="badge">{t('settings.disconnected')}</span>{/if}
                <button class="btn icon ghost small" aria-label={t('settings.moveUp')} disabled={index === 0} onclick={() => shift(index, -1)}><ArrowUp size={13} strokeWidth={1.5} /></button>
                <button class="btn icon ghost small" aria-label={t('settings.moveDown')} disabled={index === mixer.output_priority.length - 1} onclick={() => shift(index, 1)}><ArrowDown size={13} strokeWidth={1.5} /></button>
                <button class="btn icon ghost small" aria-label={t('settings.remove')} onclick={() => setPriority(mixer.output_priority.filter((n) => n !== name))}><X size={13} strokeWidth={1.5} /></button>
              </li>
            {:else}
              <li class="empty">{t('settings.priorityEmpty')}</li>
            {/each}
          </ol>
          {#if unlisted.length}
            <Dropdown
              label={t('settings.addOutput')}
              options={[{ value: '', label: t('settings.addOutput') }, ...unlisted]}
              value={''}
              width="280px"
              onchange={(name) => name && setPriority([...mixer.output_priority, name])}
            />
          {/if}
        </div>
      </section>
    {/if}

    {#if daemon.hotkeys}
      <section>
        <h2 class="hud-label">{t('settings.hotkeys.title')}</h2>
        <Hotkeys />
      </section>
    {/if}

    <section>
      <h2 class="hud-label">{t('settings.doctor')}</h2>
      <Doctor {onnavigate} onmakedefault={() => (confirmDefault = true)} />
    </section>

    <section>
      <h2 class="hud-label">{t('settings.about')}</h2>
      <div class="row">
        <p class="name">{t('settings.version')}</p>
        <span class="value">UI {__APP_VERSION__} · {t('settings.daemonVersion')} {version}</span>
      </div>
      <div class="row">
        <p class="name">{t('settings.licenses')}</p>
        <p class="hint licenses">{t('settings.licenseList')}</p>
      </div>
    </section>
  </div>
</div>

<Dialog open={confirmQuit} title={t('settings.quitConfirmTitle')} onclose={() => (confirmQuit = false)}>
  <p class="confirm">{daemon.windows ? t('settings.quitConfirmWindows') : t('settings.quitConfirm')}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (confirmQuit = false)}>{t('common.cancel')}</button>
    <button
      class="btn danger on"
      onclick={() => {
        confirmQuit = false;
        void daemon.run(daemon.client.call('daemon.quit'));
      }}>{t('settings.quit')}</button
    >
  {/snippet}
</Dialog>

<Dialog open={confirmDefault} title={t('settings.makeDefaultTitle')} onclose={() => (confirmDefault = false)}>
  <p class="confirm">{t('settings.makeDefaultConfirm')}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (confirmDefault = false)}>{t('common.cancel')}</button>
    <button class="btn invert" onclick={makeDefault}>{t('settings.makeDefault')}</button>
  {/snippet}
</Dialog>

<style>
  .screen {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .top {
    display: flex;
    align-items: center;
    min-height: 59px;
    padding: 14px 24px;
    border-bottom: 1px solid var(--line);
  }

  h1 {
    color: var(--ink);
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    display: grid;
    align-content: start;
    gap: 24px;
    max-width: 820px;
  }

  section {
    border: 1px solid var(--line);
    background: var(--bg-raised);
  }

  h2 {
    padding: 12px 24px;
    border-bottom: 1px solid var(--line);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 14px 24px;
  }

  .row + .row {
    border-top: 1px solid var(--line);
  }

  .name {
    color: var(--ink);
  }

  .hint {
    font-size: 13px;
    color: var(--muted);
  }

  .row.column {
    flex-direction: column;
    align-items: stretch;
  }

  .priority {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 4px;
  }

  .priority li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .priority .n {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--accent-ink);
    width: 16px;
  }

  .priority .device {
    flex: 1;
    color: var(--ink);
  }

  .priority .missing .device {
    color: var(--faint);
  }

  .priority .empty {
    font-size: 13px;
    color: var(--muted);
  }

  .confirm {
    color: var(--ink);
  }

  .licenses {
    text-align: right;
    max-width: 460px;
  }
</style>
