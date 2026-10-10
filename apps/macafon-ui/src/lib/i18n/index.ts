// i18n layer: every UI string goes through `t` so locales can be added later (PRD §3.2).

import { en } from './en';

type Leaves<T, Prefix extends string = ''> = {
  [K in keyof T & string]: T[K] extends string ? `${Prefix}${K}` : Leaves<T[K], `${Prefix}${K}.`>;
}[keyof T & string];

export type MessageKey = Leaves<typeof en>;

const messages: unknown = en;

/** `t('daemon.connected', { version })` — `{name}` placeholders are replaced. */
export function t(key: MessageKey, values?: Record<string, string | number>): string {
  let node = messages;
  for (const part of key.split('.')) {
    node = (node as Record<string, unknown>)[part];
  }
  let text = typeof node === 'string' ? node : key;
  if (values) {
    for (const [name, value] of Object.entries(values)) {
      text = text.replaceAll(`{${name}}`, String(value));
    }
  }
  return text;
}
