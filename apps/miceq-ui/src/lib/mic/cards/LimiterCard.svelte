<script lang="ts">
  // 08 Limiter (§6.3.8) and the output gain after it (§6.3.9).
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import Segmented from '../../components/Segmented.svelte';
  import { LIMITER_LOOKAHEAD_MS } from '../../ipc/dsp';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import type { ChainSettings } from '../../ipc/protocol';

  let { settings }: { settings: ChainSettings } = $props();
  const info = MODULES.limiter;
  const l = $derived(settings.limiter);
</script>

<ModuleCard {info} enabled={l.enabled} onenabled={(on) => daemon.edit((s) => (s.limiter.enabled = on))}>
  {#snippet badges()}
    {#if l.enabled && l.lookahead_ms > 0}<span class="badge">+{l.lookahead_ms} ms</span>{/if}
  {/snippet}
  <Param
    spec="limiter_ceiling_db"
    layout="stacked"
    label={t('params.ceiling')}
    color={info.color}
    value={l.ceiling_db}
    hint={t(info.hint)}
    onchange={(v) => daemon.edit((s) => (s.limiter.ceiling_db = v))}
  />
  {#snippet advanced()}
    <Param spec="limiter_release_ms" label={t('params.release')} hint={t('hints.release')} color={info.color} value={l.release_ms} onchange={(v) => daemon.edit((s) => (s.limiter.release_ms = v))} />
    <Segmented
      label={t('params.lookahead')}
      hint={t('hints.lookahead')}
      color={info.color}
      value={l.lookahead_ms}
      options={LIMITER_LOOKAHEAD_MS.map((ms) => ({ value: ms, label: `${ms} ms` }))}
      onchange={(v) => daemon.edit((s) => (s.limiter.lookahead_ms = v))}
    />
    <Param spec="output_gain_db" label={t('params.outputGain')} hint={t('hints.outputGain')} color={info.color} value={settings.output_gain_db} onchange={(v) => daemon.edit((s) => (s.output_gain_db = v))} />
  {/snippet}
</ModuleCard>
