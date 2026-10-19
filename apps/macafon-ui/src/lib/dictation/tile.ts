// What the Dictation tile of the tray panel and the hotkey overlay shows (PRD §9.9.2).

import type { DictationState, HotkeysState } from '../ipc/protocol';
import { displayShortcut } from '../settings/shortcut';
import { t } from '../i18n';
import { percent } from './format';

export interface DictationTile {
  on: boolean;
  /** No model yet: the tile opens the app instead of toggling. */
  needsModel: boolean;
  state: string;
  /** 0…100 while a model downloads. */
  progress: number | null;
}

export function dictationTile(dictation: DictationState | null, hotkeys: HotkeysState | null): DictationTile | null {
  if (!dictation || !dictation.prefs.show_in_overlay) return null;
  const downloading = dictation.models.find((model) => model.status === 'downloading');
  const progress = downloading ? percent(downloading.downloaded_bytes, downloading.size_bytes) : null;
  const needsModel = !dictation.models.some((model) => model.status === 'ready');
  const shortcut = hotkeys?.bindings.find((binding) => binding.action === 'dictation_hold')?.shortcut ?? null;
  let state: string;
  if (progress !== null) state = t('quick.dictationDownloading', { percent: progress });
  else if (needsModel) state = t('quick.dictationGetModel');
  else if (!dictation.prefs.enabled) state = t('quick.off');
  else if (shortcut) state = t('quick.dictationHold', { shortcut: displayShortcut(shortcut) });
  else state = t('quick.on');
  return { on: dictation.prefs.enabled, needsModel, state, progress };
}
