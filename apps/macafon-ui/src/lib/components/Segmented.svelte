<script lang="ts" generics="T">
  // Radio group of square segments (slopes, engines, modes).
  import type { Snippet } from 'svelte';
  import { tip } from './tooltip.svelte';

  interface Props {
    label: string;
    options: { value: T; label: string }[];
    value: T;
    onchange: (value: T) => void;
    color?: string;
    hint?: string;
    disabled?: boolean;
    layout?: 'row' | 'inline';
    /** Options wrap onto more lines as separate chips instead of one strip (long lists). */
    wrap?: boolean;
    extra?: Snippet;
  }

  let { label, options, value, onchange, color = 'blue', hint, disabled = false, layout = 'row', wrap = false, extra }: Props =
    $props();

  function onkeydown(event: KeyboardEvent) {
    const index = options.findIndex((o) => o.value === value);
    const delta = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!delta || disabled) return;
    event.preventDefault();
    const nextIndex = (index + delta + options.length) % options.length;
    const next = options[nextIndex];
    if (!next) return;
    onchange(next.value);
    // Focus follows the selection, or Space would activate the previously focused option.
    const group = event.currentTarget as HTMLElement;
    group.querySelectorAll<HTMLElement>('[role="radio"]')[nextIndex]?.focus();
  }
</script>

<div class="segmented {layout}" class:wrap {@attach tip(hint)} style:--c="var(--app-{color})" style:--c-ink="var(--app-{color}-ink)">
  {#if layout === 'row'}<span class="hud-label">{label}</span>{/if}
  <div class="options" role="radiogroup" aria-label={label} tabindex="-1" {onkeydown}>
    {#each options as option (option.label)}
      <button
        role="radio"
        aria-checked={option.value === value}
        tabindex={option.value === value ? 0 : -1}
        class:selected={option.value === value}
        {disabled}
        onclick={() => onchange(option.value)}>{option.label}</button
      >
    {/each}
  </div>
  {#if extra}{@render extra()}{/if}
</div>

<style>
  .segmented.row {
    display: grid;
    grid-template-columns: 104px auto 1fr;
    align-items: center;
    gap: var(--space-2);
  }

  .segmented.row.wrap {
    grid-template-columns: 104px minmax(0, 1fr);
    align-items: start;
  }

  .segmented.row.wrap > .hud-label {
    padding-top: 5px;
  }

  /* A narrow card: the label goes above the options. */
  @container (max-width: 300px) {
    .segmented.row,
    .segmented.row.wrap {
      grid-template-columns: minmax(0, 1fr);
      gap: var(--space-1);
    }

    .segmented.row > .options {
      justify-self: start;
      max-width: 100%;
    }

    .segmented.row.wrap > .hud-label {
      padding-top: 0;
    }
  }

  .wrap .options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    min-width: 0;
    border: 0;
    background: none;
  }

  .wrap button,
  .wrap button + button {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .wrap button.selected {
    border-color: var(--c);
  }

  .segmented.inline {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .options {
    display: inline-flex;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  button {
    flex-shrink: 0;
    white-space: nowrap;
    height: 22px;
    padding: 0 10px;
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
    transition:
      box-shadow var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  button + button {
    border-left: 1px solid var(--line);
  }

  /* The chosen option: its colour and a rule under it, not a fill (UIP-005). */
  button.selected {
    color: var(--c-ink);
    box-shadow: inset 0 -2px 0 var(--c);
  }

  button:hover:not(.selected):not(:disabled) {
    color: var(--ink);
  }

  button:disabled {
    color: var(--faint);
  }
</style>
