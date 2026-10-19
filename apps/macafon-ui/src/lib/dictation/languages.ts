// Dictation languages (§8.4.4): the user's keyboard layouts first, every Whisper language on
// request. The daemon lists both; nothing here limits the choice.
import type { DictationState } from '../ipc/protocol';

export interface LanguageOption {
  value: string;
  label: string;
}

/** The English name of a language code, or the code itself when the daemon does not list it. */
export function languageName(dictation: DictationState | null, code: string): string {
  const all = [...(dictation?.layout_languages ?? []), ...(dictation?.supported_languages ?? [])];
  return all.find((language) => language.code === code)?.name ?? code.toUpperCase();
}

/** The languages of the user's keyboard layouts, plus the fixed one if it is another. */
export function layoutLanguageOptions(dictation: DictationState | null): LanguageOption[] {
  const layouts = dictation?.layout_languages ?? [];
  const options = layouts.map((language) => ({ value: language.code, label: language.name }));
  const fixed = dictation?.prefs.fixed_language;
  if (fixed && !options.some((option) => option.value === fixed)) {
    options.push({ value: fixed, label: languageName(dictation, fixed) });
  }
  if (options.length === 0) options.push({ value: 'en', label: languageName(dictation, 'en') });
  return options;
}

/** Every language Whisper knows that is not already among the layout ones, by name. */
export function otherLanguageOptions(dictation: DictationState | null): LanguageOption[] {
  const shown = new Set(layoutLanguageOptions(dictation).map((option) => option.value));
  return (dictation?.supported_languages ?? [])
    .filter((language) => !shown.has(language.code))
    .map((language) => ({ value: language.code, label: language.name }))
    .sort((a, b) => a.label.localeCompare(b.label));
}
