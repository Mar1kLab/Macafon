// Sizes and times on the Dictation screen.

const MB = 1024 * 1024;
const GB = 1024 * MB;

/** `548 MB`, `1.1 GB`; binary units with the familiar short names, like Explorer. */
export function formatBytes(bytes: number): string {
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
  return `${Math.max(0, Math.round(bytes / 1024))} KB`;
}

/** `0.9 s`, `12 s`. */
export function formatSeconds(ms: number): string {
  const seconds = ms / 1000;
  return seconds < 10 ? `${seconds.toFixed(1)} s` : `${Math.round(seconds)} s`;
}

/** `12:41` in the user's locale. */
export function formatClock(unixMs: number): string {
  return new Date(unixMs).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export function percent(done: number, total: number): number {
  return total > 0 ? Math.min(100, Math.floor((done / total) * 100)) : 0;
}
