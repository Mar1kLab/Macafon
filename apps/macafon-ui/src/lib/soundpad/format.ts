// Soundpad list helpers: times, sorting and filtering, kept out of the components to test them.
import type { SoundInfo } from '../ipc/protocol';

/** `m:ss`, or `h:mm:ss` from an hour; tenths below ten seconds for short effects. */
export function formatDuration(ms: number | null, tenths = false): string {
  if (ms === null || !Number.isFinite(ms)) return '–:––';
  const total = Math.max(0, ms) / 1000;
  if (tenths && total < 10) return `0:0${total.toFixed(1)}`;
  const seconds = Math.floor(total);
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = String(seconds % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

export type SoundSort = 'name' | 'plays' | 'duration' | 'category';
/** `null` is every sound, `''` sounds outside categories, `*` favorites. */
export type SoundScope = string | null;
export const FAVORITES = '*';
export const UNCATEGORIZED = '';

export function inScope(sound: SoundInfo, scope: SoundScope): boolean {
  if (scope === null) return true;
  if (scope === FAVORITES) return sound.favorite;
  if (scope === UNCATEGORIZED) return sound.category === null;
  return sound.category === scope;
}

/** Every word of the query appears in the name or the category, in any case. */
export function matches(sound: SoundInfo, query: string): boolean {
  const haystack = `${sound.name} ${sound.category ?? ''}`.toLocaleLowerCase();
  return query
    .toLocaleLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => haystack.includes(word));
}

export function sortSounds(sounds: readonly SoundInfo[], sort: SoundSort): SoundInfo[] {
  const byName = (a: SoundInfo, b: SoundInfo) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
  const compare: Record<SoundSort, (a: SoundInfo, b: SoundInfo) => number> = {
    name: byName,
    plays: (a, b) => b.play_count - a.play_count || byName(a, b),
    duration: (a, b) => (a.duration_ms ?? Infinity) - (b.duration_ms ?? Infinity) || byName(a, b),
    category: (a, b) => (a.category ?? '').localeCompare(b.category ?? '', undefined, { sensitivity: 'base' }) || byName(a, b),
  };
  return [...sounds].sort(compare[sort]);
}
