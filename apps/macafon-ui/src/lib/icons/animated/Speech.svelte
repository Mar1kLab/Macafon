<script lang="ts">
  // A speaking head whose sound waves draw one after another (Moving Icons `speech`).
  import type { AnimatedIconProps } from './types';

  let { size = 20, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();
</script>

<svg class="moving-icon" class:animate width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path d="M8.8 20v-4.1l1.9.2a2.3 2.3 0 0 0 2.164-2.1V8.3A5.37 5.37 0 0 0 2 8.25c0 2.8.656 3.054 1 4.55a5.77 5.77 0 0 1 .029 2.758L2 20" />
  <path class="wave one" d="M17 15a3.5 3.5 0 0 0-.025-4.975" />
  <path class="wave two" d="M19.8 17.8a7.5 7.5 0 0 0 .003-10.603" />
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }

  .animate .wave,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg .wave {
    animation: wave calc(var(--dur-card) * 2) var(--ease) backwards;
  }

  .animate .one,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg .one {
    animation-delay: var(--dur-reveal);
  }

  .animate .two,
  :global(button:not(:disabled):is(:hover, :focus-visible)) svg .two {
    animation-delay: var(--dur-card);
  }

  @keyframes wave {
    from {
      opacity: 0;
    }
  }
</style>
