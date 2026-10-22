<script lang="ts">
  // 03 Noise suppression (§6.3.3): engine with its honest latency and CPU cost (NS-003),
  // intensity, NS activity (MTR-003).
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import Segmented from '../../components/Segmented.svelte';
  import Toggle from '../../components/Toggle.svelte';
  import ReductionMeter from '../../components/ReductionMeter.svelte';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import { LITE_LATENCY_SAMPLES, SAMPLE_RATE_HZ, STUDIO_LATENCY_SAMPLES } from '../../ipc/dsp';
  import { samplesToMs } from '../../dsp/format';
  import type { ChainSettings, NoiseEngine } from '../../ipc/protocol';

  let { settings }: { settings: ChainSettings } = $props();
  const info = MODULES.noise_suppression;
  const ns = $derived(settings.noise_suppression);
  const engines: { value: NoiseEngine; label: string }[] = [
    { value: 'lite', label: t('engines.lite') },
    { value: 'studio', label: t('engines.studio') },
  ];
</script>

<ModuleCard {info} enabled={ns.enabled} onenabled={(on) => daemon.edit((s) => (s.noise_suppression.enabled = on))}>
  <Segmented
    layout="inline"
    label={t('params.engine')}
    color={info.color}
    value={ns.engine}
    options={engines}
    onchange={(v) => daemon.edit((s) => (s.noise_suppression.engine = v))}
  >
    {#snippet extra()}
      <span class="badge">+{Math.round(samplesToMs(ns.engine === 'lite' ? LITE_LATENCY_SAMPLES : STUDIO_LATENCY_SAMPLES, SAMPLE_RATE_HZ))} ms</span>
      <span class="badge">{ns.engine === 'lite' ? t('engines.liteCpu') : t('engines.studioCpu')}</span>
    {/snippet}
  </Segmented>
  <Param
    spec="noise_intensity_pct"
    layout="stacked"
    label={t('params.intensity')}
    color={info.color}
    value={ns.intensity_pct}
    hint={t(info.hint)}
    onchange={(v) => daemon.edit((s) => (s.noise_suppression.intensity_pct = v))}
  />
  <ReductionMeter meterKey="noise_activity" color={info.color} activity active={ns.enabled} />
  {#snippet advanced()}
    <Param
      spec="voice_threshold_pct"
      label={t('params.voiceThreshold')}
      hint={t('hints.voiceThreshold')}
      color={info.color}
      value={ns.voice_threshold_pct}
      disabled={ns.engine !== 'lite'}
      onchange={(v) => daemon.edit((s) => (s.noise_suppression.voice_threshold_pct = v))}
    />
    <div class="switch-row" title={t('hints.postFilter')}>
      <span class="hud-label">{t('params.postFilter')}</span>
      <Toggle
        checked={ns.post_filter}
        label={t('params.postFilter')}
        color={info.color}
        disabled={ns.engine !== 'studio'}
        onchange={(on) => daemon.edit((s) => (s.noise_suppression.post_filter = on))}
      />
    </div>
  {/snippet}
</ModuleCard>

<style>
  .switch-row {
    display: grid;
    grid-template-columns: 104px auto;
    align-items: center;
    gap: var(--space-2);
  }
</style>
