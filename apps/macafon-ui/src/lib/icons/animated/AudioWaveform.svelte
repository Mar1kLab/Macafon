<script lang="ts">
  // Waveform that redraws from left to right (Macafon, in the Moving Icons manner).
  import type { AnimatedIconProps } from './types';

  let { size = 20, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path pathLength="1" d="M2 13a2 2 0 0 0 2-2V7a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0V4a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0v-4a2 2 0 0 1 2-2" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  .animate path,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg path {
    stroke-dasharray: 1;
    animation: draw calc(var(--dur-card) * 2) var(--ease-out);
  }

  @keyframes draw {
    from {
      stroke-dashoffset: 1;
    }
    to {
      stroke-dashoffset: 0;
    }
  }
</style>
