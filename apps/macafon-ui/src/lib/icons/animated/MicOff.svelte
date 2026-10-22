<script lang="ts">
  // Crossed-out microphone that shakes once (Moving Icons `mic-off`).
  import type { AnimatedIconProps } from './types';

  let { size = 13, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <line x1="2" x2="22" y1="2" y2="22" />
  <path d="M18.89 13.23A7.12 7.12 0 0 0 19 12v-2" />
  <path d="M5 10v2a7 7 0 0 0 12 5" />
  <path d="M15 9.34V5a3 3 0 0 0-5.68-1.33" />
  <path d="M9 9v3a3 3 0 0 0 5.12 2.12" />
  <line x1="12" x2="12" y1="19" y2="22" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  svg.animate,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg {
    animation: shake calc(var(--dur-card) * 2) ease-in-out;
  }

  @keyframes shake {
    17%,
    50% {
      transform: translateX(-7%);
    }
    33%,
    67% {
      transform: translateX(7%);
    }
  }
</style>
