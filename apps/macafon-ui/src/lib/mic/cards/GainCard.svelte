<script lang="ts">
  // 01 Gain (§6.3.1): input gain, raw level before Macafon with the clip warning (MIC-G1).
  import { Wand } from '@lucide/svelte';
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import LevelMeter from '../../components/LevelMeter.svelte';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import type { ChainSettings } from '../../ipc/protocol';

  let { settings, oncalibrate }: { settings: ChainSettings; oncalibrate: () => void } = $props();
  const info = MODULES.gain;
  let clipped = $state(false);
</script>

<ModuleCard {info} enabled={settings.gain.enabled} onenabled={(on) => daemon.edit((s) => (s.gain.enabled = on))}>
  <Param
    spec="gain_db"
    layout="stacked"
    label={t('params.gain')}
    color={info.color}
    value={settings.gain.gain_db}
    hint={t('hints.gain')}
    onchange={(v) => daemon.edit((s) => (s.gain.gain_db = v))}
  />
  <div class="row">
    <div class="raw">
      <LevelMeter label={t('mic.mic')} hint={t('mic.micHint')} peakKey="raw_peak_dbfs" clipKey="raw_peak_dbfs" color={info.color} onclip={(c) => (clipped = c)} />
    </div>
    <button class="btn small" onclick={oncalibrate} title={t('params.autoHint')}>
      <Wand size={12} strokeWidth={1.5} />{t('params.auto')}
    </button>
  </div>
  {#if clipped}
    <p class="clip-hint">{t('mic.clipHint')}</p>
  {/if}
</ModuleCard>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .raw {
    flex: 1;
    min-width: 0;
  }

  .clip-hint {
    font-size: 12px;
    color: var(--danger);
  }
</style>
