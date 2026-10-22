<script lang="ts">
  // The one screen header (UIP-003/004): 59 px, the HUD title, the screen's controls, a spacer and
  // its actions. `lead` goes before the title (Settings' Back). Narrow windows wrap the actions
  // onto a second row instead of squeezing them.
  import type { Snippet } from 'svelte';

  let { title, lead, children, actions }: { title: string; lead?: Snippet; children?: Snippet; actions?: Snippet } = $props();
</script>

<header class="page-header">
  {@render lead?.()}
  <h1 class="hud-label">{title}</h1>
  {@render children?.()}
  <span class="spacer"></span>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</header>

<style>
  .page-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    min-height: 59px;
    padding: var(--space-3) var(--space-6);
    border-bottom: 1px solid var(--line);
  }

  h1 {
    margin-right: var(--space-2);
    color: var(--ink);
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
