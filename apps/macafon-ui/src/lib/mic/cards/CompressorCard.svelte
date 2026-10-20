<script lang="ts">
  // 06 Compressor (§6.3.6): Amount macro; auto makeup keeps A/B about character, not level.
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import Segmented from '../../components/Segmented.svelte';
  import Toggle from '../../components/Toggle.svelte';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import { compressorAmount, compressorFromAmount, nearestCompressorAmount } from '../../dsp/macros';
  import { SPECS } from '../../ipc/dsp';
  import type { ChainSettings, CompressorDetector } from '../../ipc/protocol';

  let { settings }: { settings: ChainSettings } = $props();
  const info = MODULES.compressor;
  const c = $derived(settings.compressor);
  const amount = $derived(compressorAmount(c));
  const detectors: { value: CompressorDetector; label: string }[] = [
    { value: 'peak', label: t('params.peak') },
    { value: 'rms', label: t('params.rms') },
  ];
</script>

<ModuleCard
  {info}
  enabled={c.enabled}
  onenabled={(on) => daemon.edit((s) => (s.compressor.enabled = on))}
  custom={amount === null}
  onreset={() => daemon.edit((s) => Object.assign(s.compressor, compressorFromAmount(nearestCompressorAmount(s.compressor))))}
>
  <Param
    spec="compressor_amount_pct"
    layout="stacked"
    label={t('params.amount')}
    color={info.color}
    value={amount ?? nearestCompressorAmount(c)}
    disabled={amount === null}
    hint={t(info.hint)}
    onchange={(v) => daemon.edit((s) => Object.assign(s.compressor, compressorFromAmount(v)))}
  />
  {#snippet advanced()}
    <Param spec="compressor_threshold_db" label={t('params.threshold')} hint={t('hints.threshold')} color={info.color} value={c.threshold_db} onchange={(v) => daemon.edit((s) => (s.compressor.threshold_db = v))} />
    <Param spec="compressor_ratio" label={t('params.ratio')} hint={t('hints.ratio')} color={info.color} value={c.ratio} onchange={(v) => daemon.edit((s) => (s.compressor.ratio = v))} />
    <Param spec="compressor_attack_ms" label={t('params.attack')} hint={t('hints.attack')} color={info.color} value={c.attack_ms} onchange={(v) => daemon.edit((s) => (s.compressor.attack_ms = v))} />
    <Param spec="compressor_release_ms" label={t('params.release')} hint={t('hints.release')} color={info.color} value={c.release_ms} onchange={(v) => daemon.edit((s) => (s.compressor.release_ms = v))} />
    <Param spec="compressor_knee_db" label={t('params.knee')} hint={t('hints.knee')} color={info.color} value={c.knee_db} onchange={(v) => daemon.edit((s) => (s.compressor.knee_db = v))} />
    <div class="makeup">
      <Param
        spec="compressor_makeup_db"
        label={t('params.makeup')}
        hint={t('hints.makeup')}
        color={info.color}
        value={c.makeup_db ?? SPECS.compressor_makeup_db.default}
        disabled={c.makeup_db === null}
        format={c.makeup_db === null ? () => t('params.auto') : undefined}
        onchange={(v) => daemon.edit((s) => (s.compressor.makeup_db = v))}
      />
      <span class="auto">
        <span class="hud-label">{t('params.auto')}</span>
        <Toggle
          checked={c.makeup_db === null}
          label="{t('params.makeup')} {t('params.auto')}"
          color={info.color}
          onchange={(on) => daemon.edit((s) => (s.compressor.makeup_db = on ? null : SPECS.compressor_makeup_db.default))}
        />
      </span>
    </div>
    <Segmented
      label={t('params.detector')}
      hint={t('hints.detector')}
      color={info.color}
      value={c.detector}
      options={detectors}
      onchange={(v) => daemon.edit((s) => (s.compressor.detector = v))}
    />
  {/snippet}
</ModuleCard>

<style>
  .makeup {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 10px;
    align-items: center;
  }

  /* The ruler keeps a usable width; on a narrow card Auto goes under it. */
  .makeup > :global(:first-child) {
    flex: 1 1 200px;
    min-width: 0;
  }

  .auto {
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
