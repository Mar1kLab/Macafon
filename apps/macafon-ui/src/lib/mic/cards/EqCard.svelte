<script lang="ts">
  // 05 EQ (§6.3.5): character + Amount in simple mode (EQ-002), the graph for direct edits
  // (EQ-001); editing a band makes the EQ custom.
  import { Maximize2, Minimize2, Trash2 } from '@lucide/svelte';
  import ModuleCard from '../../components/ModuleCard.svelte';
  import Param from '../../components/Param.svelte';
  import Dropdown from '../../components/Dropdown.svelte';
  import EqGraph from '../../components/EqGraph.svelte';
  import { MODULES } from '../modules';
  import { daemon } from '../../state';
  import { t } from '../../i18n';
  import { eqCharacter, eqFromCharacter, type EqCharacterId } from '../../dsp/macros';
  import { EQ_CHARACTERS, SPECS } from '../../ipc/dsp';
  import type { ChainSettings, EqFilter } from '../../ipc/protocol';
  import { tip } from '../../components/tooltip.svelte';

  let { settings, expanded, onexpand }: { settings: ChainSettings; expanded: boolean; onexpand: () => void } =
    $props();
  const info = MODULES.eq;
  const eq = $derived(settings.eq);
  const macro = $derived(eqCharacter(eq));
  let selected = $state<number | null>(null);
  const band = $derived(selected === null ? undefined : eq.bands[selected]);

  type Choice = { value: EqCharacterId | 'custom'; label: string };
  const characters: Choice[] = EQ_CHARACTERS.map((c) => ({ value: c.id, label: t(`eq.characters.${c.id}`) }));
  const choices = $derived<Choice[]>(macro ? characters : [{ value: 'custom', label: t('module.custom') }, ...characters]);
  const FILTERS: EqFilter[] = ['bell', 'low_shelf', 'high_shelf', 'low_cut', 'high_cut', 'notch'];
  const hasGain = (filter: EqFilter) => filter === 'bell' || filter === 'low_shelf' || filter === 'high_shelf';

  function setCharacter(id: EqCharacterId, amount: number) {
    daemon.edit((s) => Object.assign(s.eq, eqFromCharacter(id, amount)));
  }
</script>

<ModuleCard
  {info}
  wide
  enabled={eq.enabled}
  onenabled={(on) => daemon.edit((s) => (s.eq.enabled = on))}
  custom={macro === null}
  onreset={() => setCharacter('flat', SPECS.eq_amount_pct.default)}
>
  {#snippet badges()}
    <button class="btn icon ghost small expand" aria-label={expanded ? t('eq.collapse') : t('eq.expand')} {@attach tip(expanded ? t('eq.collapse') : t('eq.expand'))} onclick={onexpand}>
      {#if expanded}<Minimize2 size={13} strokeWidth={1.5} />{:else}<Maximize2 size={13} strokeWidth={1.5} />{/if}
    </button>
  {/snippet}
  <div class="simple">
    <div class="character">
      <span class="hud-label">{t('params.character')}</span>
      <Dropdown
        label={t('params.character')}
        options={choices}
        value={macro?.character ?? 'custom'}
        onchange={(id: Choice['value']) => id !== 'custom' && setCharacter(id, macro?.amountPct ?? SPECS.eq_amount_pct.default)}
      />
    </div>
    <div class="amount">
      <Param
        spec="eq_amount_pct"
        label={t('params.amount')}
        color={info.color}
        value={macro?.amountPct ?? SPECS.eq_amount_pct.default}
        disabled={macro === null || macro.character === 'flat'}
        onchange={(v) => macro && setCharacter(macro.character, v)}
      />
    </div>
  </div>
  <EqGraph
    bands={eq.bands}
    outputGainDb={eq.output_gain_db}
    color={info.color}
    disabled={!eq.enabled}
    height={expanded ? 240 : 112}
    {selected}
    onselect={(index) => (selected = index)}
    onchange={(index, next) => daemon.edit((s) => (s.eq.bands[index] = next))}
    onadd={(next) => {
      daemon.edit((s) => s.eq.bands.push(next));
      selected = eq.bands.length;
    }}
    ondelete={(index) => {
      daemon.edit((s) => s.eq.bands.splice(index, 1));
      selected = null;
    }}
  />
  {#snippet advanced()}
    {#if band && selected !== null}
      {@const index = selected}
      <div class="band-head">
        <span class="hud-label">{t('eq.band', { n: index + 1 })}</span>
        <div class="type">
          <Dropdown
            label={t('params.type')}
            options={FILTERS.map((f) => ({ value: f, label: t(`eq.filters.${f}`) }))}
            value={band.filter}
            onchange={(filter) => daemon.edit((s) => (s.eq.bands[index]!.filter = filter))}
          />
        </div>
        <button class="btn icon ghost small" aria-label={t('eq.deleteBand')} {@attach tip(t('eq.deleteBand'))} onclick={() => {
          daemon.edit((s) => s.eq.bands.splice(index, 1));
          selected = null;
        }}><Trash2 size={13} strokeWidth={1.5} /></button>
      </div>
      <Param spec="eq_freq_hz" label={t('params.frequency')} color={info.color} value={band.freq_hz} onchange={(v) => daemon.edit((s) => (s.eq.bands[index]!.freq_hz = v))} />
      <Param spec="eq_gain_db" label={t('params.gain')} color={info.color} value={band.gain_db} disabled={!hasGain(band.filter)} onchange={(v) => daemon.edit((s) => (s.eq.bands[index]!.gain_db = v))} />
      <Param spec="eq_q" label={t('params.q')} color={info.color} value={band.q} onchange={(v) => daemon.edit((s) => (s.eq.bands[index]!.q = v))} />
    {:else}
      <p class="help">{t('eq.graphHelp')}</p>
    {/if}
    <Param spec="eq_output_gain_db" label={t('params.outputGain')} color={info.color} value={eq.output_gain_db} onchange={(v) => daemon.edit((s) => (s.eq.output_gain_db = v))} />
  {/snippet}
</ModuleCard>

<style>
  .simple {
    display: grid;
    grid-template-columns: minmax(150px, 220px) 1fr;
    gap: var(--space-4);
    align-items: center;
  }

  .character {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-2);
    align-items: center;
  }

  .amount :global(.ruler.row) {
    grid-template-columns: auto 1fr 52px;
  }

  .expand {
    color: var(--muted);
  }

  .band-head {
    display: grid;
    grid-template-columns: 104px 160px auto;
    gap: var(--space-2);
    align-items: center;
  }

  .help {
    font-size: var(--text-sm);
    color: var(--muted);
  }
</style>
