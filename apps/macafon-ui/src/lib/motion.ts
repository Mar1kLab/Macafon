// Motion tokens for script-driven animation (design system §5). The CSS side lives in
// tokens.css; these mirror it for `svelte/transition` and `svelte/motion`, which need numbers.
// Chromium maps Windows "Animation effects" off to `prefers-reduced-motion`, so one check covers both.
import { prefersReducedMotion } from 'svelte/motion';
import { cubicOut } from 'svelte/easing';

export const DURATION = {
  /** Hover, focus, colour changes (`--dur`). */
  fast: 150,
  /** Tooltips, menus, step changes (`--dur-reveal`). */
  reveal: 200,
  /** Card appearance, expanding sections (`--dur-card`). */
  card: 300,
} as const;

/** Spring presets for `Spring`: `snappy` for values that follow a pointer, `soft` for reveals. */
export const SPRING = {
  snappy: { stiffness: 0.3, damping: 0.8 },
  soft: { stiffness: 0.12, damping: 0.6 },
} as const;

/** A duration that drops to 0 under reduced motion. */
export function motion(ms: number): number {
  return prefersReducedMotion.current ? 0 : ms;
}

/** Options for `slide`/`fade`/`fly` with a token duration and the shared easing. */
export function reveal(kind: keyof typeof DURATION = 'reveal') {
  return { duration: motion(DURATION[kind]), easing: cubicOut };
}
