// The hotkey overlay's setup (§9.9): what it shows, where side panels open, how it is placed.
import { cubicInOut, cubicOut } from 'svelte/easing';
import type { TransitionConfig } from 'svelte/transition';
import type { OverlayExpand, OverlayPrefs } from '../ipc/protocol';

/** Until a daemon without `[ui.overlay]` answers: everything the overlay always had. */
export const DEFAULT_OVERLAY: OverlayPrefs = {
  enabled: true,
  order: ['volumes', 'mic_switch', 'processing_switch', 'mixer_switch', 'dictation', 'sounds', 'output_device', 'mic_level', 'preset'],
  volumes: true,
  mic_switch: true,
  processing_switch: true,
  mixer_switch: true,
  preset: true,
  output_device: false,
  mic_level: false,
  expand: 'right',
  placement: 'anchored',
};

/** A side panel grows out of the overlay: its width (or height, opening down) goes from zero
 * to its own and back, so the window follows it smoothly both ways, and its content fades. */
export function reveal(node: HTMLElement, { expand, duration }: { expand: OverlayExpand; duration: number }): TransitionConfig {
  const down = expand === 'down';
  const size = down ? node.offsetHeight : node.offsetWidth;
  return {
    duration,
    easing: cubicInOut,
    css: (t) => `overflow: hidden; ${down ? 'height' : 'width'}: ${t * size}px; opacity: ${cubicOut(t)};`,
  };
}
