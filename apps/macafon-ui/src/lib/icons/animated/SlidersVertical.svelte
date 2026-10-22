<script lang="ts">
  // Three faders whose knobs slide to new positions (Moving Icons `sliders-vertical`), driven
  // by one tween instead of per-line animation frames.
  import { Tween, prefersReducedMotion } from 'svelte/motion';
  import { cubicInOut } from 'svelte/easing';
  import { DURATION, motion } from '../../motion';
  import type { AnimatedIconProps } from './types';

  let { size = 20, strokeWidth = 1.5, animate = false }: AnimatedIconProps = $props();

  const progress = new Tween(0, { easing: cubicInOut });

  $effect(() => {
    void progress.set(animate && !prefersReducedMotion.current ? 1 : 0, { duration: motion(DURATION.card + 100) });
  });

  const at = (from: number, to: number) => from + (to - from) * progress.current;
  // Knob of each fader at rest (Lucide's shape) and moved. The track breaks for 4 units: above
  // the knob on the outer faders, below it on the middle one.
  const knobs = $derived([at(14, 9), at(8, 14), at(16, 8)]);
</script>

<svg class="moving-icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={strokeWidth} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#each [5, 12, 19] as x, index (x)}
    {@const y = knobs[index]!}
    {@const above = index === 1 ? 0 : 4}
    <line x1={x} x2={x} y1="3" y2={y - above} />
    <line x1={x} x2={x} y1={y + 4 - above} y2="21" />
    <line x1={x - 2} x2={x + 2} y1={y} y2={y} />
  {/each}
</svg>

<style>
  svg {
    display: block;
    overflow: visible;
  }
</style>
