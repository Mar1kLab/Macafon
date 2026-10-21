<script lang="ts">
  // Module card (design system §6, DSP-001): number, HUD title, on/off, main control, gain
  // reduction and an Advanced accordion. Off → content at 40% opacity, the frame stays.
  import { prefersReducedMotion } from 'svelte/motion';
  import type { Snippet } from 'svelte';
  import { slide } from 'svelte/transition';
  import { ChevronDown } from '@lucide/svelte';
  import Toggle from './Toggle.svelte';
  import Ticks from './Ticks.svelte';
  import ReductionMeter from './ReductionMeter.svelte';
  import { t } from '../i18n';

  import type { ModuleInfo } from '../mic/modules';

  interface Props {
    info: ModuleInfo;
    enabled: boolean;
    onenabled: (enabled: boolean) => void;
    /** DSP-002: Advanced settings no longer match the main slider. */
    custom?: boolean;
    onreset?: () => void;
    children: Snippet;
    advanced?: Snippet;
    badges?: Snippet;
    wide?: boolean;
    class?: string;
  }

  let {
    info,
    enabled,
    onenabled,
    custom = false,
    onreset,
    children,
    advanced,
    badges,
    wide = false,
    class: className = '',
  }: Props = $props();

  let open = $state(false);
  const title = $derived(t(info.title));
  const advancedId = $derived(`advanced-${info.id}`);
</script>

<section
  class="card {className}"
  class:off={!enabled}
  class:wide
  aria-label={title}
  style:--c="var(--app-{info.color})"
  style:--c-ink="var(--app-{info.color}-ink)"
>
  <span class="ticks"><Ticks /></span>
  <header title={t(info.hint)}>
    <span class="number">{info.number}</span>
    <h3 class="hud-label">{title}</h3>
    {#if custom}
      <span class="badge custom">{t('module.custom')}</span>
    {/if}
    {#if badges}{@render badges()}{/if}
    <span class="spacer"></span>
    <Toggle checked={enabled} label="{title} {enabled ? t('module.on') : t('module.off')}" color={info.color} onchange={onenabled} />
  </header>
  <div class="content">
    {@render children()}
    {#if custom && onreset}
      <button class="btn small reset" onclick={onreset}>{t('module.resetSimple')}</button>
    {/if}
    {#if info.reduction}
      <ReductionMeter meterKey={info.reduction} color={info.color} active={enabled} />
    {/if}
  </div>
  {#if advanced}
    <button
      class="advanced-toggle hud-label"
      aria-expanded={open}
      aria-controls={advancedId}
      onclick={() => (open = !open)}
    >
      {t('module.advanced')}
      <span class="chevron" class:open><ChevronDown size={12} strokeWidth={1.5} /></span>
    </button>
    {#if open}
      <div class="advanced content" id={advancedId} transition:slide={{ duration: prefersReducedMotion.current ? 0 : 300 }}>
        {@render advanced()}
      </div>
    {/if}
  {/if}
</section>

<style>
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    background: var(--bg-raised);
    border: 1px solid var(--line);
    transition: border-color var(--dur-card) ease-out;
    min-width: 0;
  }

  .card:hover,
  .card:focus-within {
    border-color: var(--line-strong);
  }

  .ticks {
    display: none;
  }

  .card:focus-within .ticks {
    display: block;
  }

  /* Colour strip of the module on the left edge. */
  .card::before {
    content: '';
    position: absolute;
    left: -1px;
    top: -1px;
    width: 2px;
    height: 34px;
    background: var(--c);
    transition: opacity var(--dur) var(--ease);
  }

  .off::before {
    opacity: 0.25;
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px 0 16px;
    min-height: 32px;
  }

  .number {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--c-ink);
  }

  h3 {
    color: var(--ink);
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }

  .custom {
    color: var(--app-amber-ink);
    border-color: color-mix(in srgb, var(--app-amber) 50%, transparent);
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 16px 0;
    transition: opacity var(--dur) var(--ease);
  }

  .off .content {
    opacity: 0.4;
  }

  .reset {
    align-self: flex-start;
  }

  .advanced-toggle {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
    margin-top: auto;
    padding: 6px 14px 8px;
    font-size: 10px;
    color: var(--faint);
  }

  .advanced-toggle:hover,
  .advanced-toggle[aria-expanded='true'] {
    color: var(--ink);
  }

  .chevron {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }

  .chevron.open {
    transform: rotate(180deg);
  }

  .advanced {
    border-top: 1px solid var(--line);
    padding: 14px 16px 16px;
    gap: 10px;
  }

  .card:not(:has(.advanced-toggle)) .content:last-child {
    padding-bottom: 16px;
  }
</style>
