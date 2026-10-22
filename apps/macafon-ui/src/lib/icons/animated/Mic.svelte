<script lang="ts">
  // Microphone that bobs once, as if picked up (Macafon, in the Moving Icons manner).
  import type { AnimatedIconProps } from './types';

  let { size = 20, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <g class="body">
    <rect x="9" y="2" width="6" height="13" rx="3" />
    <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
  </g>
  <path d="M12 19v3" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  .body {
    transition: transform var(--dur-card) cubic-bezier(0.175, 0.885, 0.32, 1.275);
  }

  .animate .body,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg .body {
    transform: translateY(-1.5px);
  }
</style>
