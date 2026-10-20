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

/** Reserve the final layout before revealing it. The native window grows once on entry
 * and shrinks after exit; clipping never feeds intermediate dimensions back through IPC. */
export function reveal(node: HTMLElement, { expand, duration }: { expand: OverlayExpand; duration: number }): TransitionConfig {
  const { width, height } = node.getBoundingClientRect();
  return {
    duration,
    easing: cubicInOut,
    css: (t) => {
      const hidden = (1 - t) * 100;
      const inset = expand === 'down' ? `0 0 ${hidden}% 0` : expand === 'left' ? `0 0 0 ${hidden}%` : `0 ${hidden}% 0 0`;
      return `width: ${width}px; height: ${height}px; overflow: hidden; clip-path: inset(${inset}); opacity: ${cubicOut(t)};`;
    },
  };
}
