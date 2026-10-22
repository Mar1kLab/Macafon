<script lang="ts">
  // A tick that draws itself (Moving Icons `check`).
  import type { AnimatedIconProps } from './types';

  let { size = 13, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path pathLength="1" d="M4 12l5 5L20 6" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  .animate path,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg path {
    stroke-dasharray: 1;
    animation: draw var(--dur-card) var(--ease-out) backwards;
  }

  @keyframes draw {
    from {
      stroke-dashoffset: 1;
      opacity: 0;
    }
  }
</style>
