<script lang="ts">
  // Channel drawer (PRD §9.4): EQ with the channel presets (CFX-001/002), the FX of §7.3
  // (CFX-003/004) with one-click bypass (CFX-007), and the channel itself — name, colour,
  // icon, default (CH-002/003).
  import { radioKeys } from '../components/radioKeys';
  import { X } from '@lucide/svelte';
  import EqGraph from '../components/EqGraph.svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import Param from '../components/Param.svelte';
  import Toggle from '../components/Toggle.svelte';
  import Ticks from '../components/Ticks.svelte';
  import Dialog from '../components/Dialog.svelte';
  import ReductionMeter from '../components/ReductionMeter.svelte';
  import { mixerMeterBus } from '../state/meters';
  import { CHANNEL_ICON } from './icons';
  import { daemon } from '../state';
  import { compressorAmount, compressorFromAmount, nearestCompressorAmount } from '../dsp/macros';
  import { CHANNEL_COLORS, CHANNEL_EQ_PRESETS, CHANNEL_ICONS } from '../ipc/dsp';
  import { t } from '../i18n';
  import type { ChannelEqPreset, ChannelSettings, ChannelState } from '../ipc/protocol';
  import Field from '../components/Field.svelte';
  import Input from '../components/Input.svelte';

  export type Panel = 'eq' | 'fx' | 'channel';

  interface Props {
    channel: ChannelState;
    panel: Panel;
    isDefault: boolean;
    onpanel: (panel: Panel) => void;
    onclose: () => void;
  }

  let { channel, panel, isDefault, onpanel, onclose }: Props = $props();
  let selected = $state<number | null>(null);
  let confirmDelete = $state(false);
  const s = $derived(channel.settings);
  const amount = $derived(compressorAmount(s.compressor));
  const PANELS = $derived<Panel[]>(daemon.channelFx ? ['eq', 'fx', 'channel'] : ['channel']);
  /** Without channel effects (WIN-CH-006) every panel request shows the channel settings. */
  const shown = $derived<Panel>(daemon.channelFx ? panel : 'channel');

  const edit = (change: (settings: ChannelSettings) => void) => daemon.editChannel(channel.id, change);
  /** A band edit leaves the preset (CFX-002): the dropdown shows Custom. */
  const editEq = (change: (settings: ChannelSettings) => void) =>
    edit((settings) => {
      change(settings);
      settings.eq_preset = null;
    });

  type PresetChoice = { value: ChannelEqPreset | 'custom'; label: string; group?: string };
  const presetChoices = $derived<PresetChoice[]>([
    ...(s.eq_preset === null ? [{ value: 'custom' as const, label: t('module.custom') }] : []),
    ...CHANNEL_EQ_PRESETS.map((preset) => ({
      value: preset.id,
      label: preset.name,
      group: t(`mixer.eqCategories.${preset.category}`),
    })),
  ]);

  function loadPreset(id: ChannelEqPreset) {
    const preset = CHANNEL_EQ_PRESETS.find((p) => p.id === id);
    if (!preset) return;
    selected = null;
    edit((settings) => {
      settings.eq.bands = structuredClone(preset.eq.bands);
      settings.eq.output_gain_db = preset.eq.output_gain_db;
      settings.eq_preset = id;
    });
  }

  function rename(event: Event) {
    const name = (event.currentTarget as HTMLInputElement).value.trim();
    if (name && name !== channel.name) {
      void daemon.setMixer(daemon.client.call('channel.update', { id: channel.id, patch: { name } }));
    }
  }
</script>

<aside class="drawer" aria-label={t('mixer.drawer', { name: channel.name })} style:--c="var(--app-{channel.color})">
  <Ticks />
  <header>
    <h2>{channel.name}</h2>
    <div class="tabs" role="tablist">
      {#each PANELS as item (item)}
        <button role="tab" aria-selected={shown === item} class:selected={shown === item} onclick={() => onpanel(item)}>
          {t(`mixer.panels.${item}`)}
        </button>
      {/each}
    </div>
    <button class="btn icon ghost" aria-label={t('common.close')} onclick={onclose}><X size={16} strokeWidth={1.5} /></button>
  </header>

  <div class="body">
    {#if shown === 'eq'}
      <div class="row">
        <span class="hud-label">{t('mixer.eq')}</span>
        <Toggle checked={s.eq.enabled} color={channel.color} label={t('mixer.eq')} onchange={(on) => edit((x) => (x.eq.enabled = on))} />
      </div>
      <div class="row">
        <span class="hud-label">{t('mixer.preset')}</span>
        <div class="grow">
          <Dropdown
            label={t('mixer.preset')}
            options={presetChoices}
            value={s.eq_preset ?? 'custom'}
            onchange={(id: PresetChoice['value']) => id !== 'custom' && loadPreset(id)}
          />
        </div>
      </div>
      <EqGraph
        bands={s.eq.bands}
        outputGainDb={s.eq.output_gain_db}
        color={channel.color}
        disabled={!s.eq.enabled || s.bypass}
        height={200}
        {selected}
        onselect={(index) => (selected = index)}
        onchange={(index, band) => editEq((x) => (x.eq.bands[index] = band))}
        onadd={(band) => {
          editEq((x) => x.eq.bands.push(band));
          selected = s.eq.bands.length;
        }}
        ondelete={(index) => {
          editEq((x) => x.eq.bands.splice(index, 1));
          selected = null;
        }}
      />
      <p class="help">{t('eq.graphHelp')}</p>
    {:else if shown === 'fx'}
      <div class="row module">
        <div>
          <p class="name">{t('mixer.bypass')}</p>
          <p class="help">{t('mixer.bypassHint')}</p>
        </div>
        <Toggle checked={s.bypass} label={t('mixer.bypass')} onchange={(on) => edit((x) => (x.bypass = on))} />
      </div>
      <section class="module" class:off={!s.noise_suppression.enabled || s.bypass}>
        <div class="row">
          <p class="name">{t('modules.noise_suppression.title')}</p>
          <Toggle checked={s.noise_suppression.enabled} color="violet" label={t('modules.noise_suppression.title')} onchange={(on) => edit((x) => (x.noise_suppression.enabled = on))} />
        </div>
        <p class="help">{t('mixer.nsHint')}</p>
        <Param spec="noise_intensity_pct" label={t('params.intensity')} color="violet" value={s.noise_suppression.intensity_pct} onchange={(v) => edit((x) => (x.noise_suppression.intensity_pct = v))} />
      </section>
      <section class="module" class:off={!s.compressor.enabled || s.bypass}>
        <div class="row">
          <p class="name">{t('modules.compressor.title')}</p>
          <Toggle checked={s.compressor.enabled} color="amber" label={t('modules.compressor.title')} onchange={(on) => edit((x) => (x.compressor.enabled = on))} />
        </div>
        <Param
          spec="compressor_amount_pct"
          label={t('params.amount')}
          color="amber"
          value={amount ?? nearestCompressorAmount(s.compressor)}
          onchange={(v) => edit((x) => Object.assign(x.compressor, compressorFromAmount(v)))}
        />
        <ReductionMeter
          read={() => mixerMeterBus.reduction.get(channel.id)?.compressor ?? 0}
          color="amber"
          active={s.compressor.enabled && !s.bypass}
        />
      </section>
      <section class="module" class:off={!s.limiter.enabled || s.bypass}>
        <div class="row">
          <p class="name">{t('modules.limiter.title')}</p>
          <Toggle checked={s.limiter.enabled} color="orange" label={t('modules.limiter.title')} onchange={(on) => edit((x) => (x.limiter.enabled = on))} />
        </div>
        <Param spec="limiter_ceiling_db" label={t('params.ceiling')} color="orange" value={s.limiter.ceiling_db} onchange={(v) => edit((x) => (x.limiter.ceiling_db = v))} />
        <ReductionMeter
          read={() => mixerMeterBus.reduction.get(channel.id)?.limiter ?? 0}
          color="orange"
          active={s.limiter.enabled && !s.bypass}
        />
      </section>
    {:else}
      <Field label={t('mixer.name')}>
        <Input value={channel.name} maxlength={32} onchange={rename} onkeydown={(e) => e.key === 'Enter' && (e.currentTarget as HTMLInputElement).blur()} />
      </Field>
      <div class="field">
        <span class="hud-label">{t('mixer.color')}</span>
        <div class="swatches" role="radiogroup" aria-label={t('mixer.color')} use:radioKeys>
          {#each CHANNEL_COLORS as color (color)}
            <button
              role="radio"
              aria-checked={channel.color === color}
              tabindex={channel.color === color ? 0 : -1}
              aria-label={color}
              class:selected={channel.color === color}
              style:--s="var(--app-{color})"
              onclick={() => daemon.setMixer(daemon.client.call('channel.update', { id: channel.id, patch: { color } }))}
            ></button>
          {/each}
        </div>
      </div>
      <div class="field">
        <span class="hud-label">{t('mixer.icon')}</span>
        <div class="icons" role="radiogroup" aria-label={t('mixer.icon')} use:radioKeys>
          {#each CHANNEL_ICONS as icon, i (icon)}
            {@const Icon = CHANNEL_ICON[icon as keyof typeof CHANNEL_ICON]}
            <button
              role="radio"
              aria-checked={channel.icon === icon}
              tabindex={channel.icon === icon || (!CHANNEL_ICONS.includes(channel.icon ?? '') && i === 0) ? 0 : -1}
              aria-label={icon}
              class:selected={channel.icon === icon}
              onclick={() => daemon.setMixer(daemon.client.call('channel.update', { id: channel.id, patch: { icon } }))}
            ><Icon size={15} strokeWidth={1.5} /></button>
          {/each}
        </div>
      </div>
      <div class="row module">
        <div>
          <p class="name">{t('mixer.channelSwitch')}</p>
          <p class="help">{isDefault ? t('mixer.channelSwitchDefault') : t('mixer.channelSwitchHint')}</p>
        </div>
        <Toggle
          checked={channel.enabled}
          color={channel.color}
          disabled={isDefault}
          label={t('mixer.channelSwitch')}
          onchange={(enabled) => daemon.setMixer(daemon.client.call('channel.update', { id: channel.id, patch: { enabled } }))}
        />
      </div>
      <div class="row module">
        <div>
          <p class="name">{t('mixer.makeDefault')}</p>
          <p class="help">{t('mixer.makeDefaultHint')}</p>
        </div>
        <button class="btn" disabled={isDefault || !channel.enabled} onclick={() => daemon.setMixer(daemon.client.call('channel.set_default', { id: channel.id }))}>
          {isDefault ? t('mixer.default') : t('mixer.makeDefaultAction')}
        </button>
      </div>
      <div class="row module">
        <div>
          <p class="name">{t('mixer.delete')}</p>
          <p class="help">{isDefault ? t('mixer.deleteDefault') : t('mixer.deleteHint')}</p>
        </div>
        <button class="btn danger" disabled={isDefault} onclick={() => (confirmDelete = true)}>{t('mixer.delete')}</button>
      </div>
    {/if}
  </div>
</aside>

<Dialog open={confirmDelete} title={t('mixer.deleteTitle', { name: channel.name })} onclose={() => (confirmDelete = false)}>
  <p>{t('mixer.deleteConfirm')}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (confirmDelete = false)}>{t('common.cancel')}</button>
    <button
      class="btn danger on"
      onclick={() => {
        confirmDelete = false;
        onclose();
        void daemon.setMixer(daemon.client.call('channel.delete', { id: channel.id }));
      }}>{t('mixer.delete')}</button
    >
  {/snippet}
</Dialog>

<style>
  .drawer {
    position: absolute;
    z-index: 20;
    top: 0;
    right: 0;
    bottom: 0;
    width: 440px;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--line-strong);
    background: var(--bg-raised);
    animation: slide var(--dur-card) var(--ease-out);
  }

  @keyframes slide {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 12px 12px 12px 20px;
    border-top: 3px solid var(--c);
    border-bottom: 1px solid var(--line);
  }

  h2 {
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 700;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tabs {
    display: flex;
    margin-left: auto;
    border: 1px solid var(--line-strong);
  }

  .tabs button {
    padding: 4px 10px;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .tabs button.selected {
    background: var(--invert-bg);
    color: var(--invert-fg);
  }

  .body {
    flex: 1;
    overflow-y: auto;
    display: grid;
    align-content: start;
    gap: var(--space-4);
    padding: 20px;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .grow {
    flex: 1;
    max-width: 260px;
  }

  .module {
    display: grid;
    gap: var(--space-2);
    padding: 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .row.module {
    display: flex;
  }

  .module.off :global(.ruler) {
    opacity: 0.4;
  }

  .name {
    color: var(--ink);
  }

  .help {
    font-size: var(--text-sm);
    color: var(--muted);
  }

  .field {
    display: grid;
    gap: var(--space-2);
  }



  .swatches,
  .icons {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .swatches button {
    width: 28px;
    height: 28px;
    background: var(--s);
    border: 2px solid transparent;
  }

  .swatches button.selected {
    border-color: var(--ink-strong);
  }

  .icons button {
    width: 32px;
    height: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--line);
    color: var(--muted);
  }

  .icons button.selected {
    border-color: var(--c);
    color: var(--ink-strong);
  }

  @media (prefers-reduced-motion: reduce) {
    .drawer {
      animation: none;
    }
  }
</style>
