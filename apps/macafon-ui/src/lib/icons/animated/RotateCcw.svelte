<script lang="ts">
  // An arrow that turns back a little (Moving Icons `rotate-ccw`).
  import type { AnimatedIconProps } from './types';

  let { size = 13, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
  <path d="M3 3v5h5" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  svg {
    transform-origin: center;
    transition: transform var(--dur-card) cubic-bezier(0.175, 0.885, 0.32, 1.275);
  }

  svg.animate,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg {
    transform: rotate(-50deg);
  }
</style>
