<script lang="ts">
  // Two sheets that slide apart (Moving Icons `copy`).
  import type { AnimatedIconProps } from './types';

  let { size = 13, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <rect class="front" width="14" height="14" x="8" y="8" rx="2" ry="2" />
  <path class="back" d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  .front,
  .back {
    transition: transform var(--dur-card) cubic-bezier(0.16, 1, 0.3, 1);
  }

  .animate .front,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg .front {
    transform: translate(-3px, -3px);
  }

  .animate .back,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg .back {
    transform: translate(3px, 3px);
  }
</style>
