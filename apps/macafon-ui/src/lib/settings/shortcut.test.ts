import { describe, expect, it } from 'vitest';
import { displayShortcut, recordShortcut } from './shortcut';

const key = (code: string, mods: { ctrl?: boolean; alt?: boolean; shift?: boolean; meta?: boolean } = {}) => ({
  code,
  ctrlKey: mods.ctrl ?? false,
  altKey: mods.alt ?? false,
  shiftKey: mods.shift ?? false,
  metaKey: mods.meta ?? false,
});

describe('shortcut recording', () => {
  // The same canonical strings as Accel::to_string in macafond (HK-002).
  it('produces the daemon canonical form', () => {
    expect(recordShortcut(key('KeyM', { ctrl: true, alt: true }))).toEqual({ kind: 'done', shortcut: 'Ctrl+Alt+M' });
    expect(recordShortcut(key('PageUp', { shift: true, meta: true }))).toEqual({
      kind: 'done',
      shortcut: 'Shift+Super+PageUp',
    });
    expect(recordShortcut(key('F9'))).toEqual({ kind: 'done', shortcut: 'F9' });
    expect(recordShortcut(key('ControlLeft', { ctrl: true }))).toEqual({ kind: 'partial', preview: 'Ctrl+' });
    expect(recordShortcut(key('KeyM', { shift: true })).kind).toBe('needs_modifier');
    expect(recordShortcut(key('Semicolon', { ctrl: true })).kind).toBe('unsupported');
  });

  it('records the console key like the daemon and shows it as its character', () => {
    expect(recordShortcut(key('Backquote', { ctrl: true, alt: true }))).toEqual({
      kind: 'done',
      shortcut: 'Ctrl+Alt+Backquote',
    });
    expect(recordShortcut(key('Backquote')).kind).toBe('needs_modifier');
    expect(displayShortcut('Ctrl+Alt+Backquote')).toBe('Ctrl+Alt+`');
  });
});
