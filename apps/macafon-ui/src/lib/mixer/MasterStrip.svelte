<script lang="ts">
  import HotkeyHint from '../settings/HotkeyHint.svelte';
  // Master (CH-006): fader and mute over all channels, the loudest channel meter.
  import Fader from './Fader.svelte';
  import StereoMeter from './StereoMeter.svelte';
  import { daemon } from '../state';
  import { formatVolumeDb } from '../dsp/format';
  import { t } from '../i18n';
  import type { MixerState } from '../ipc/protocol';

  let { mixer }: { mixer: MixerState } = $props();
</script>

<section class="master" aria-label={t('mixer.master')}>
  <header><h2 class="hud-label">{t('mixer.master')}</h2></header>
  <div aria-hidden="true"></div>
  <div class="level">
    <Fader
      value={mixer.master_volume_pct}
      label={t('mixer.masterVolume')}
      color="teal"
      defaultValue={100}
      onchange={(value) => daemon.setMaster({ volume_pct: value })}
    />
    <StereoMeter id="master" color="teal" />
  </div>
  <p class="readout">
    <span class="value">{Math.round(mixer.master_volume_pct)}%</span>
    {#if !daemon.windows}<span class="db">{formatVolumeDb(mixer.master_volume_pct)}</span>{/if}
  </p>
  <button
    class="btn small danger"
    class:on={mixer.master_muted}
    aria-pressed={mixer.master_muted}
    title={t('mixer.masterMute')}
    onclick={() => daemon.setMaster({ muted: !mixer.master_muted })}>M</button
  >
  <div class="shortcuts"><HotkeyHint actions={['master_volume_up', 'master_volume_down', 'master_mute']} /></div>
</section>

<style>
  .shortcuts { min-height: 16px; display: flex; justify-content: center; align-items: center; }
  .master {
    display: grid;
    grid-template-rows: auto minmax(84px, 1fr) minmax(140px, 240px) auto auto auto;
    gap: var(--space-3);
    width: 136px;
    flex: none;
    padding: 0 12px 12px;
    border: 1px solid var(--line-strong);
    border-top: 3px solid var(--app-teal);
    background: var(--bg-raised);
  }

  header {
    display: flex;
    align-items: center;
    height: 40px;
  }

  h2 {
    color: var(--ink);
  }

  .level {
    display: flex;
    justify-content: center;
    gap: var(--space-3);
    min-height: 0;
  }


  .readout {
    white-space: nowrap;
    display: flex;
    justify-content: center;
    align-items: baseline;
    gap: var(--space-2);
  }

  .value {
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--ink-strong);
  }

  .db {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--muted);
  }

  .btn {
    justify-content: center;
  }
</style>
