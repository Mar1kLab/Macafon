// Dictation history actions shared by the TRY IT panel and the overlay's history part (STT-015).

import type { DictationResult, DictationState } from '../ipc/protocol';
import { daemon } from '../state';
import { t } from '../i18n';
import { languageName } from './languages';

/** The detected language of a result, or "Auto" when the model did not report one. */
export function resultLanguage(dictation: DictationState | null, entry: DictationResult): string {
  return entry.language ? languageName(dictation, entry.language) : t('dictation.test.auto');
}

export function clearHistory() {
  return daemon.setDictation(daemon.client.call('dictation.history_clear'));
}

export function reinsert(entry: DictationResult) {
  return daemon.run(daemon.client.call('dictation.reinsert', { id: entry.id }));
}

/** Copies a result's text; `id` marks the last copied result for 1.5 s. */
export class Copier {
  id = $state<number | null>(null);

  async copy(entry: DictationResult) {
    await navigator.clipboard.writeText(entry.text);
    this.id = entry.id;
    setTimeout(() => {
      if (this.id === entry.id) this.id = null;
    }, 1500);
  }
}
