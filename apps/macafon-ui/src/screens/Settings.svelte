<script lang="ts">
  // 03 / SETTINGS (PRD §9.5) — what the daemon supports today: start with system (AS-001),
  // fallback microphone (MIC-006), virtual microphone format (MIC-011), quit (BG-005), about.
  // Hotkeys and Doctor arrive with M2.
  import Toggle from '../lib/components/Toggle.svelte';
  import Dropdown from '../lib/components/Dropdown.svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import { daemon } from '../lib/state';
  import { t } from '../lib/i18n';

  const mic = $derived(daemon.mic);
  const autostart = $derived(daemon.autostart);
  const fallbackOptions = $derived([
    { value: null as string | null, label: t('settings.none') },
    ...daemon.devices
      .filter((device) => device.name !== mic?.device)
      .map((device) => ({ value: device.name as string | null, label: device.description })),
  ]);
  let confirmQuit = $state(false);
  const version = $derived(daemon.status.state === 'connected' ? daemon.status.version : '—');

  async function setAutostart(enabled: boolean) {
    const state = await daemon.run(daemon.client.call('autostart.set', { enabled }));
    if (state) daemon.autostart = state;
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
              {t('settings.startWithSystemHint', { method: autostart.method === 'systemd' ? t('settings.methodSystemd') : t('settings.methodXdg') })}
            </p>
          {/if}
        </div>
        <Toggle checked={autostart?.enabled ?? false} disabled={!autostart} label={t('settings.startWithSystem')} onchange={setAutostart} />
      </div>
      <div class="row">
        <div>
          <p class="name">{t('settings.quit')}</p>
          <p class="hint">{t('settings.quitHint')}</p>
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
            onopen={() => daemon.refreshDevices()}
            onchange={(device) => daemon.setMic(daemon.client.call('mic.set_fallback_device', { device }))}
          />
        </div>
        <div class="row">
          <div>
            <p class="name">{t('settings.format')}</p>
            <p class="hint">{t('settings.formatHint')}</p>
          </div>
          <span class="badge">{mic.stereo ? t('settings.stereo') : t('settings.mono')} · 48 kHz · float32</span>
        </div>
      </section>
    {/if}

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
  <p class="confirm">{t('settings.quitConfirm')}</p>
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

  .confirm {
    color: var(--ink);
  }

  .licenses {
    text-align: right;
    max-width: 460px;
  }
</style>
