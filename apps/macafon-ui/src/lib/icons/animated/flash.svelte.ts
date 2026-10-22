// A short "something changed" pulse for animated icons: `on` turns true for a moment after the
// watched value changes, never for its first value, so a screen that just opened stays still.
import { DURATION } from '../../motion';

export function flashOnChange(read: () => unknown, ms: number = DURATION.card * 2) {
  let on = $state(false);
  let first = true;
  let previous: unknown;
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    // The effect also reruns when an object read() goes through is replaced (any settings
    // edit); only a different value counts.
    const value = read();
    if (first || Object.is(value, previous)) {
      first = false;
      previous = value;
      return;
    }
    previous = value;
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
