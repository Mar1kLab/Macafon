// Shortcut recording (HK-002): a key press becomes the daemon's canonical form, `Ctrl+Alt+M`.
// Mirrors `Accel` in macafond/src/hotkeys/accel.rs: the same keys, modifier order and rule
// that a bare letter or digit needs Ctrl, Alt or Super.

/** Keys the daemon accepts besides letters, digits and F-keys; `KeyboardEvent.code` names. */
const NAMED = new Set([
  'Space',
  'Insert',
  'Delete',
  'Home',
  'End',
  'PageUp',
  'PageDown',
  'ArrowUp',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'Pause',
  'ScrollLock',
  'PrintScreen',
]);

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/;

export type Recorded =
  /** Only modifiers are down so far; `preview` is what is held, `Ctrl+Alt+`. */
  | { kind: 'partial'; preview: string }
  | { kind: 'done'; shortcut: string }
  | { kind: 'needs_modifier'; preview: string }
  | { kind: 'unsupported'; preview: string };

type KeyLike = Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>;

function keyName(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1]!;
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1]!;
  const fn = /^F([0-9]{1,2})$/.exec(code);
  if (fn && Number(fn[1]) >= 1 && Number(fn[1]) <= 24) return code;
  return NAMED.has(code) ? code : null;
}

/** Physical keys (`code`), so the shortcut does not depend on the keyboard layout. */
export function recordShortcut(event: KeyLike): Recorded {
  const prefix = [
    event.ctrlKey && 'Ctrl',
    event.altKey && 'Alt',
    event.shiftKey && 'Shift',
    event.metaKey && 'Super',
  ]
    .filter(Boolean)
    .map((name) => `${name}+`)
    .join('');
  if (MODIFIER_CODES.test(event.code)) return { kind: 'partial', preview: prefix };
  const key = keyName(event.code);
  if (!key) return { kind: 'unsupported', preview: `${prefix}${event.code}` };
  const shortcut = `${prefix}${key}`;
  if (key.length === 1 && !(event.ctrlKey || event.altKey || event.metaKey)) {
    return { kind: 'needs_modifier', preview: shortcut };
  }
  return { kind: 'done', shortcut };
}
