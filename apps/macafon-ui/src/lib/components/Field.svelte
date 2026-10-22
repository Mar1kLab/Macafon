<script lang="ts">
  // A labelled form row (UIP-003): HUD label, the control, an optional one-line hint.
  // A <label> when it wraps one control, a <div> for a group (`group`), so clicks still focus.
  import type { Snippet } from 'svelte';

  let { label, hint, group = false, grow = false, children }: { label: string; hint?: string; group?: boolean; grow?: boolean; children: Snippet } = $props();
</script>

<svelte:element this={group ? 'div' : 'label'} class="ui-field" class:grow role={group ? 'group' : undefined} aria-label={group ? label : undefined}>
  <span class="hud-label">{label}</span>
  {@render children()}
  {#if hint}<span class="hint">{hint}</span>{/if}
</svelte:element>

<style>
  .ui-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .grow {
    flex: 1;
  }

  .hint {
    font-size: var(--text-sm);
    color: var(--muted);
  }
</style>
