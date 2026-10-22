<script lang="ts">
  // Monitor and phone; the phone slides in beside it (Macafon, in the Moving Icons manner).
  import type { AnimatedIconProps } from './types';

  let { size = 20, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path d="M18 8V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h8" />
  <path d="M10 19v-3.96 3.15" />
  <path d="M7 19h5" />
  <rect class="phone" width="6" height="10" x="16" y="12" rx="2" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  .phone {
    transition: transform var(--dur-card) cubic-bezier(0.175, 0.885, 0.32, 1.275);
  }

  .animate .phone,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg .phone {
    transform: translate(1px, -2px);
  }
</style>
