// The hotkey overlay's setup (§9.9): what it shows, where side panels open, how it is placed.
import { cubicOut } from 'svelte/easing';
import type { TransitionConfig } from 'svelte/transition';
import type { OverlayExpand, OverlayPrefs } from '../ipc/protocol';

/** Until a daemon without `[ui.overlay]` answers: everything the overlay always had. */
export const DEFAULT_OVERLAY: OverlayPrefs = {
  enabled: true,
  order: ['volumes', 'mic_switch', 'processing_switch', 'mixer_switch', 'dictation', 'dictation_history', 'sounds', 'display_profiles', 'output_device', 'mic_level', 'preset'],
  volumes: true,
  mic_switch: true,
  processing_switch: true,
  mixer_switch: true,
  dictation_history: true,
  display_profiles: true,
  preset: true,
  output_device: false,
  mic_level: false,
  expand: 'right',
  placement: 'anchored',
};

/** A side panel takes its whole (fixed) size at once, so the window resizes a single time,
 * and only its content moves: it fades in sliding out of the overlay, and back. Nothing about
 * the layout changes during the animation, so nothing inside reflows or jumps. */
export function reveal(_node: HTMLElement, { expand, duration }: { expand: OverlayExpand; duration: number }): TransitionConfig {
  const [x, y] = expand === 'down' ? [0, -10] : expand === 'left' ? [14, 0] : [-14, 0];
  return {
    duration,
    easing: cubicOut,
    css: (t, u) => `opacity: ${t}; transform: translate(${u * x}px, ${u * y}px);`,
  };
}
