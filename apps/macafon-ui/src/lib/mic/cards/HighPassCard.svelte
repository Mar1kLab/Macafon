<script lang="ts">
  // 02 High-pass (§6.3.2): frequency on the main slider, slope in Advanced.
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import Segmented from '../../components/Segmented.svelte';
  import { HIGH_PASS_SLOPES_DB_OCT } from '../../ipc/dsp';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import type { ChainSettings } from '../../ipc/protocol';

  let { settings }: { settings: ChainSettings } = $props();
  const info = MODULES.high_pass;
</script>

<ModuleCard {info} enabled={settings.high_pass.enabled} onenabled={(on) => daemon.edit((s) => (s.high_pass.enabled = on))}>
  <Param
    spec="high_pass_freq_hz"
    layout="stacked"
    label={t('params.frequency')}
    color={info.color}
    value={settings.high_pass.freq_hz}
    hint={t(info.hint)}
    onchange={(v) => daemon.edit((s) => (s.high_pass.freq_hz = v))}
  />
  {#snippet advanced()}
    <Segmented
      label={t('params.slope')}
      hint={t('hints.slope')}
      color={info.color}
      value={settings.high_pass.slope_db_oct}
      options={HIGH_PASS_SLOPES_DB_OCT.map((slope) => ({ value: slope, label: `${slope} dB/oct` }))}
      onchange={(v) => daemon.edit((s) => (s.high_pass.slope_db_oct = v))}
    />
  {/snippet}
</ModuleCard>
