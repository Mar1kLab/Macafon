// Synthetic envelopes are deterministic so browser previews remain repeatable.
export const SILENT = -120;

/** Deterministic pseudo-random sequence so screenshots are stable. */
export function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

/** Speech-like envelope in dBFS: syllables around −24 dBFS, pauses at the room floor. */
export function voiceLevel(timeS: number, floorDbfs: number): number {
  const phrase = timeS % 6;
  if (phrase > 4.2) return floorDbfs + Math.sin(timeS * 31) * 1.5;
  const syllable = Math.max(0, Math.sin(timeS * Math.PI * 4.3)) ** 0.6;
  const accent = 0.6 + 0.4 * Math.sin(timeS * 1.7);
  const level = -42 + 20 * syllable * accent;
  return Math.max(level, floorDbfs);
}

