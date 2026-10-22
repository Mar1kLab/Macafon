// A short "something changed" pulse for animated icons: `on` turns true for a moment after the
// watched value changes, never for its first value, so a screen that just opened stays still.
import { DURATION } from '../../motion';

export function flashOnChange(read: () => unknown, ms: number = DURATION.card * 2) {
  let on = $state(false);
  let first = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    read();
    if (first) {
      first = false;
      return;
    }
    on = false;
    clearTimeout(timer);
    // A frame off first, so a keyframe animation restarts on a quick second change.
    requestAnimationFrame(() => {
      on = true;
      timer = setTimeout(() => (on = false), ms);
    });
  });
  $effect(() => () => clearTimeout(timer));
  return {
    get on() {
      return on;
    },
  };
}
