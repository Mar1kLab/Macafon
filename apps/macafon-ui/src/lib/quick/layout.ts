// How the hotkey overlay lays its parts out (§9.9), shared by the overlay and the layout editor
// in Settings: the parts go in the saved order; Volumes is a block beside the controls column,
// on the left when it comes before every shown control, else on the right; in the column,
// switches next to each other share rows two by two and the other parts take a row each.
import type { OverlayItem, OverlayPrefs } from '../ipc/protocol';

export const OVERLAY_PARTS: OverlayItem[] = [
  'volumes',
  'mic_switch',
  'processing_switch',
  'mixer_switch',
  'dictation',
  'dictation_history',
  'sounds',
  'display_profiles',
  'output_device',
  'mic_level',
  'preset',
];

const SWITCHES = new Set<OverlayItem>(['mic_switch', 'processing_switch', 'mixer_switch', 'dictation', 'dictation_history', 'sounds', 'display_profiles']);

export const isSwitch = (part: OverlayItem) => SWITCHES.has(part);

/** The saved order without unknown or repeated parts, missing ones after it in default order. */
export function overlayOrder(order: readonly OverlayItem[] | undefined): OverlayItem[] {
  const seen = new Set<OverlayItem>();
  for (const part of [...(order ?? []), ...OVERLAY_PARTS]) {
    if (OVERLAY_PARTS.includes(part)) seen.add(part);
  }
  return [...seen];
}

/** Whether a part is on. Dictation and Sounds keep their own `show_in_overlay`; a part whose
 * feature is missing (no dictation, no SoundDeck, no display profiles) is never on. */
export function partShown(
  setup: OverlayPrefs,
  part: OverlayItem,
  extra: { dictation: boolean | null; sounds: boolean | null; displays?: boolean },
): boolean {
  switch (part) {
    case 'dictation':
      return extra.dictation ?? false;
    case 'dictation_history':
      return setup.dictation_history && extra.dictation !== null;
    case 'sounds':
      return extra.sounds ?? false;
    case 'display_profiles':
      return setup.display_profiles && (extra.displays ?? false);
    default:
      return setup[part];
  }
}

export type OverlayRow = { kind: 'switches'; parts: OverlayItem[] } | { kind: 'wide'; part: OverlayItem };

/** The controls column: runs of switches share grid rows, everything else gets its own row. */
export function overlayRows(parts: readonly OverlayItem[]): OverlayRow[] {
  const rows: OverlayRow[] = [];
  for (const part of parts) {
    if (part === 'volumes') continue;
    const last = rows.at(-1);
    if (isSwitch(part) && last?.kind === 'switches') last.parts.push(part);
    else rows.push(isSwitch(part) ? { kind: 'switches', parts: [part] } : { kind: 'wide', part });
  }
  return rows;
}

/** Volumes sits left of the controls when it comes before all of them. */
export function volumesFirst(parts: readonly OverlayItem[]): boolean {
  const index = parts.indexOf('volumes');
  return index >= 0 && parts.slice(0, index).every((part) => part === 'volumes');
}

/** `part` moved next to `target` (before or after it) in the full order. */
export function movePart(order: readonly OverlayItem[], part: OverlayItem, target: OverlayItem | 'start' | 'end', after = false): OverlayItem[] {
  const rest = order.filter((item) => item !== part);
  if (target === 'start') return [part, ...rest];
  if (target === 'end' || target === part) return target === part ? [...order] : [...rest, part];
  const index = rest.indexOf(target);
  if (index < 0) return [...rest, part];
  rest.splice(after ? index + 1 : index, 0, part);
  return rest;
}
