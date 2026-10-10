<script lang="ts">
  // 07 De-esser (§6.3.7): Intensity macro; Listen solos the sidechain band.
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import Segmented from '../../components/Segmented.svelte';
  import Toggle from '../../components/Toggle.svelte';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import { deEsserFromIntensity, deEsserIntensity, nearestDeEsserIntensity } from '../../dsp/macros';
  import type { ChainSettings, DeEsserMode } from '../../ipc/protocol';

  let { settings }: { settings: ChainSettings } = $props();
  const info = MODULES.de_esser;
  const d = $derived(settings.de_esser);
  const intensity = $derived(deEsserIntensity(d));
  const modes: { value: DeEsserMode; label: string }[] = [
    { value: 'split_band', label: t('params.splitBand') },
    { value: 'wideband', label: t('params.wideband') },
  ];

  function apply(s: ChainSettings, pct: number) {
    Object.assign(s.de_esser, { ...deEsserFromIntensity(pct), listen: s.de_esser.listen });
  }
</script>

<ModuleCard
  {info}
  enabled={d.enabled}
  onenabled={(on) => daemon.edit((s) => (s.de_esser.enabled = on))}
  custom={intensity === null}
  onreset={() => daemon.edit((s) => apply(s, nearestDeEsserIntensity(s.de_esser)))}
>
  {#snippet badges()}
    {#if d.listen}<span class="badge listen">{t('params.listen')}</span>{/if}
  {/snippet}
  <Param
    spec="de_esser_intensity_pct"
    layout="stacked"
    label={t('params.intensity')}
    color={info.color}
    value={intensity ?? nearestDeEsserIntensity(d)}
    disabled={intensity === null}
    hint={t(info.hint)}
    onchange={(v) => daemon.edit((s) => apply(s, v))}
  />
  {#snippet advanced()}
    <Param spec="de_esser_freq_hz" label={t('params.frequency')} hint={t('hints.frequency')} color={info.color} value={d.freq_hz} onchange={(v) => daemon.edit((s) => (s.de_esser.freq_hz = v))} />
    <Param spec="de_esser_threshold_db" label={t('params.threshold')} hint={t('hints.threshold')} color={info.color} value={d.threshold_db} onchange={(v) => daemon.edit((s) => (s.de_esser.threshold_db = v))} />
    <Param spec="de_esser_max_reduction_db" label={t('params.maxReduction')} hint={t('hints.maxReduction')} color={info.color} value={d.max_reduction_db} onchange={(v) => daemon.edit((s) => (s.de_esser.max_reduction_db = v))} />
    <Segmented label={t('params.mode')} hint={t('hints.mode')} color={info.color} value={d.mode} options={modes} onchange={(v) => daemon.edit((s) => (s.de_esser.mode = v))} />
    <div class="switch-row" title={t('hints.listen')}>
      <span class="hud-label">{t('params.listen')}</span>
      <Toggle checked={d.listen} label={t('params.listen')} color={info.color} onchange={(on) => daemon.edit((s) => (s.de_esser.listen = on))} />
    </div>
  {/snippet}
</ModuleCard>

<style>
  .listen {
    color: var(--app-pink-ink);
  }

  .switch-row {
    display: grid;
    grid-template-columns: 104px auto;
    align-items: center;
    gap: 10px;
  }
</style>
