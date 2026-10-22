<script lang="ts">
  // The shared empty, first-run and error panel (UIP-003): corner ticks, a HUD label, an Archivo
  // headline with one Instrument Serif word, the body and the actions. `compact` is the inline
  // version for an empty list inside a section.
  import type { Snippet } from 'svelte';
  import Ticks from './Ticks.svelte';

  interface Props {
    label: string;
    title?: string;
    /** The serif accent word after the title, and the punctuation that closes the headline. */
    accent?: string;
    end?: string;
    width?: string;
    compact?: boolean;
    tone?: 'default' | 'danger';
    role?: 'alert' | 'status';
    icon?: Snippet;
    children?: Snippet;
    actions?: Snippet;
  }

  let { label, title, accent, end = '.', width = '560px', compact = false, tone = 'default', role, icon, children, actions }: Props = $props();
</script>

<section class="empty-state" class:compact class:danger={tone === 'danger'} style:--width={width} {role}>
  {#if !compact}<Ticks color={tone === 'danger' ? 'var(--danger)' : undefined} />{/if}
  <span class="label">
    {#if icon}<span class="icon" aria-hidden="true">{@render icon()}</span>{/if}
    <span class="hud-label">{label}</span>
  </span>
  {#if title}
    <h2>{title}{#if accent}{' '}<em>{accent}</em>{/if}{end}</h2>
  {/if}
  {#if children}<div class="body">{@render children()}</div>{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</section>

<style>
  .empty-state {
    position: relative;
    display: grid;
    gap: var(--space-4);
    justify-items: start;
    width: min(var(--width), 100%);
    padding: var(--space-8);
    border: 1px solid var(--line);
    background: var(--bg-raised);
    animation: empty-in var(--dur-card) var(--ease-out);
  }

  .compact {
    width: 100%;
    gap: var(--space-2);
    padding: var(--space-6);
    border-style: dashed;
    background: none;
    justify-items: center;
    text-align: center;
  }

  .label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .icon {
    display: inline-flex;
    color: var(--muted);
  }

  .danger .hud-label,
  .danger .icon {
    color: var(--danger);
  }

  h2 {
    font-family: var(--font-display);
    font-size: var(--text-display);
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1.1;
    color: var(--ink-strong);
  }

  .compact h2 {
    font-size: var(--text-lg);
    letter-spacing: -0.02em;
  }

  em {
    font-family: var(--font-serif);
    font-weight: 400;
    letter-spacing: 0;
  }

  .body {
    display: grid;
    gap: var(--space-3);
    width: 100%;
    color: var(--ink);
  }

  .compact .body {
    color: var(--muted);
    font-size: var(--text-sm);
  }

  .body :global(.note) {
    font-size: var(--text-md);
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  @keyframes empty-in {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
