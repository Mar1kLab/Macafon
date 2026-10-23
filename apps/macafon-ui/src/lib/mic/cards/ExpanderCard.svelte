<script lang="ts">
  // 04 Expander / gate (§6.3.4): Sensitivity macro relative to the measured noise floor.
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import Toggle from '../../components/Toggle.svelte';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import {
    expanderFromSensitivity,
    expanderReferenceDbfs,
    expanderSensitivity,
    nearestExpanderSensitivity,
  } from '../../dsp/macros';
  import type { ChainSettings, MicState } from '../../ipc/protocol';

  let { mic }: { mic: MicState } = $props();
  const info = MODULES.expander;
  const settings = $derived(mic.settings);
  const ex = $derived(settings.expander);
  const sensitivity = $derived(expanderSensitivity(ex));
  const reference = $derived(expanderReferenceDbfs(settings, mic.noise_floor_dbfs));

  function applySensitivity(s: ChainSettings, pct: number) {
    Object.assign(s.expander, expanderFromSensitivity(pct, expanderReferenceDbfs(s, mic.noise_floor_dbfs)));
  }
</script>

<ModuleCard
  {info}
  enabled={ex.enabled}
  onenabled={(on) => daemon.edit((s) => (s.expander.enabled = on))}
  custom={sensitivity === null}
  onreset={() => daemon.edit((s) => applySensitivity(s, nearestExpanderSensitivity(s.expander)))}
>
  <Param
    spec="expander_sensitivity_pct"
    layout="stacked"
    label={t('params.sensitivity')}
    color={info.color}
    value={sensitivity ?? nearestExpanderSensitivity(ex)}
    disabled={sensitivity === null}
    hint={t(info.hint)}
    onchange={(v) => daemon.edit((s) => applySensitivity(s, v))}
  />
  {#snippet advanced()}
    <Param
      spec="expander_threshold_db"
      label={t('params.threshold')}
      hint={t('hints.threshold')}
      color={info.color}
      value={ex.threshold_db}
      defaultValue={expanderFromSensitivity(50, reference).threshold_db}
      onchange={(v) => daemon.edit((s) => (s.expander.threshold_db = v))}
    />
    <div class="ratio">
      <Param
        spec="expander_ratio"
        label={t('params.ratio')}
        hint={t('hints.ratio')}
        color={info.color}
        value={ex.ratio === 'gate' ? 10 : ex.ratio}
        disabled={ex.ratio === 'gate'}
        format={(v) => (ex.ratio === 'gate' ? '1:∞' : `1:${v.toFixed(1)}`)}
        onchange={(v) => daemon.edit((s) => (s.expander.ratio = v))}
      />
      <span class="gate">
        <span class="hud-label">{t('params.gate')}</span>
        <Toggle
          checked={ex.ratio === 'gate'}
          label={t('params.gate')}
          color={info.color}
          onchange={(on) => daemon.edit((s) => (s.expander.ratio = on ? 'gate' : 3))}
        />
      </span>
    </div>
    <Param spec="expander_range_db" label={t('params.range')} hint={t('hints.range')} color={info.color} value={ex.range_db} onchange={(v) => daemon.edit((s) => (s.expander.range_db = v))} />
    <Param spec="expander_attack_ms" label={t('params.attack')} hint={t('hints.attack')} color={info.color} value={ex.attack_ms} onchange={(v) => daemon.edit((s) => (s.expander.attack_ms = v))} />
    <Param spec="expander_hold_ms" label={t('params.hold')} hint={t('hints.hold')} color={info.color} value={ex.hold_ms} onchange={(v) => daemon.edit((s) => (s.expander.hold_ms = v))} />
    <Param spec="expander_release_ms" label={t('params.release')} hint={t('hints.release')} color={info.color} value={ex.release_ms} onchange={(v) => daemon.edit((s) => (s.expander.release_ms = v))} />
    <Param spec="expander_hysteresis_db" label={t('params.hysteresis')} hint={t('hints.hysteresis')} color={info.color} value={ex.hysteresis_db} onchange={(v) => daemon.edit((s) => (s.expander.hysteresis_db = v))} />
  {/snippet}
</ModuleCard>

<style>
  .ratio {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-2);
    align-items: center;
  }

  /* A narrow card: Gate goes under the ratio slider instead of squeezing it. */
  @container (max-width: 300px) {
    .ratio {
      grid-template-columns: minmax(0, 1fr);
    }

    .gate {
      justify-self: end;
    }
  }

  .gate {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
</style>
