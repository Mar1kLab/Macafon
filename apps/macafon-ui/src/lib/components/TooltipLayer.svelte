<script lang="ts">
  // Draws the page's one tooltip (UIP-002): a HUD label in a 1 px frame with corner ticks, the
  // shortcut as a key. Bits UI places it against the hovered or focused element and keeps it
  // inside the window.
  import { Tooltip } from 'bits-ui';
  import { TOOLTIP_ID, tooltip } from './tooltip.svelte';
</script>

<Tooltip.Provider delayDuration={0} disableHoverableContent>
  <Tooltip.Root open={tooltip.open} onOpenChange={(open) => !open && tooltip.hide()}>
    {#if tooltip.anchor}
      <Tooltip.Portal>
        <Tooltip.Content
          id={TOOLTIP_ID}
          class="sketch-tooltip"
          role="tooltip"
          customAnchor={tooltip.anchor}
          side="top"
          sideOffset={8}
          collisionPadding={8}
        >
          <span class="text">{tooltip.text}</span>
          {#if tooltip.shortcut}<kbd>{tooltip.shortcut}</kbd>{/if}
        </Tooltip.Content>
      </Tooltip.Portal>
    {/if}
  </Tooltip.Root>
</Tooltip.Provider>

<style>
  :global(.sketch-tooltip) {
    z-index: 1000;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 320px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--line-strong);
    background: var(--bg-sunken);
    /* Corner ticks drawn as background so the content needs no extra elements. */
    background-image:
      linear-gradient(var(--ink-strong), var(--ink-strong)),
      linear-gradient(var(--ink-strong), var(--ink-strong)),
      linear-gradient(var(--ink-strong), var(--ink-strong)),
      linear-gradient(var(--ink-strong), var(--ink-strong));
    background-size: 5px 1px, 1px 5px, 5px 1px, 1px 5px;
    background-position: left top, left top, right bottom, right bottom;
    background-repeat: no-repeat;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.02em;
    line-height: 1.4;
    color: var(--ink);
    pointer-events: none;
    animation: tooltip-in var(--dur-reveal) var(--ease-out);
  }

  :global(.sketch-tooltip) kbd {
    flex: none;
    padding: 0 var(--space-1);
    border: 1px solid var(--line);
    background: var(--bg);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    letter-spacing: 0.08em;
    color: var(--accent-ink);
    white-space: nowrap;
  }

  @keyframes tooltip-in {
    from {
      opacity: 0;
      transform: translateY(2px);
    }
  }
</style>
