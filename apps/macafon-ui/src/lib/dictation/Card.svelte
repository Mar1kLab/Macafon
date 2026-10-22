<script lang="ts">
  // A settings card of the Dictation screen: the module card's frame (design system §6) —
  // number, HUD title, colour strip, ticks on focus — without an on/off switch.
  import type { Snippet } from 'svelte';
  import Ticks from '../components/Ticks.svelte';

  interface Props {
    number: string;
    title: string;
    hint: string;
    color: string;
    /** Content at 40 % opacity, as a disabled module (Parakeet's language card). */
    dimmed?: boolean;
    badge?: string;
    class?: string;
    children: Snippet;
    footer?: Snippet;
  }

  let { number, title, hint, color, dimmed = false, badge, class: className = '', children, footer }: Props = $props();
</script>

<section class="card {className}" class:dimmed aria-label={title} style:--c="var(--app-{color})" style:--c-ink="var(--app-{color}-ink)">
  <span class="ticks"><Ticks /></span>
  <header title={hint}>
    <span class="number">{number}</span>
    <h3 class="hud-label">{title}</h3>
    {#if badge}<span class="badge">{badge}</span>{/if}
  </header>
  <div class="content">{@render children()}</div>
  {#if footer}<footer>{@render footer()}</footer>{/if}
</section>

<style>
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--bg-raised);
    border: 1px solid var(--line);
    transition: border-color var(--dur-card) var(--ease-out);
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

  .card::before {
    content: '';
    position: absolute;
    left: -1px;
    top: -1px;
    width: 2px;
    height: 34px;
    background: var(--c);
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 32px;
    padding: 8px 14px 0 16px;
  }

  .number {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--c-ink);
  }

  h3 {
    color: var(--ink);
    white-space: nowrap;
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: 12px 16px 16px;
    transition: opacity var(--dur) var(--ease);
  }

  .dimmed .content {
    opacity: 0.4;
  }

  footer {
    margin-top: auto;
    padding: 8px 16px;
    border-top: 1px solid var(--line);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
  }
</style>
