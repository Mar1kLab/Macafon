<script lang="ts">
  // Dictation in the hotkey overlay (§9.9.2), opened by its round button like the soundpad:
  // the state and shortcuts, a quick language switch, the engine and device, and the history
  // with copy and insert-again — the things wanted mid-conversation without opening the app.
  import { Copy, CornerDownLeft, Trash2 } from '@lucide/svelte';
  import { radioKeys } from '../components/radioKeys';
  import { formatClock, formatSeconds } from '../dictation/format';
  import { displayShortcut } from '../settings/shortcut';
  import type { DictationLanguage, DictationResult } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';

  const dictation = $derived(daemon.dictation);
  const prefs = $derived(dictation?.prefs);
  const parakeet = $derived(prefs?.engine === 'parakeet');
  const model = $derived(dictation?.models.find((m) => m.id === prefs?.model)?.name ?? '—');
  let copied = $state<number | null>(null);

  type Choice = 'layout' | DictationLanguage;
  const choices: { value: Choice; label: string }[] = [
    { value: 'layout', label: t('dictation.language.layout') },
    { value: 'en', label: t('dictation.language.en') },
    { value: 'uk', label: t('dictation.language.uk') },
    { value: 'ru', label: t('dictation.language.ru') },
  ];
  const choice = $derived<Choice>(prefs?.language === 'fixed' ? prefs.fixed_language : 'layout');

  function choose(value: Choice) {
    if (value === 'layout') daemon.setDictationPrefs({ language: 'layout' });
    else daemon.setDictationPrefs({ language: 'fixed', fixed_language: value });
  }

  function shortcut(action: string): string | null {
    const value = daemon.hotkeys?.bindings.find((binding) => binding.action === action)?.shortcut;
    return value ? displayShortcut(value) : null;
  }

  async function copy(entry: DictationResult) {
    await navigator.clipboard.writeText(entry.text);
    copied = entry.id;
    setTimeout(() => {
      if (copied === entry.id) copied = null;
    }, 1500);
  }

  function language(entry: DictationResult): string {
    return entry.language ? t(`dictation.language.${entry.language}`) : t('dictation.test.auto');
  }
</script>

{#if dictation && prefs}
  <section class="dictation" id="overlay-dictation" aria-label={t('quick.dictation')}>
    <div class="bar">
      <div class="languages" role="radiogroup" aria-label={t('dictation.language.label')} tabindex="-1" use:radioKeys>
        {#each choices as option (option.value)}
          <button class="chip" class:active={choice === option.value} role="radio" aria-checked={choice === option.value} tabindex={choice === option.value ? 0 : -1} disabled={parakeet} onclick={() => choose(option.value)}>{option.label}</button>
        {/each}
      </div>
      <span class="badge">{prefs.engine.toUpperCase()} · {model.toUpperCase()} · {prefs.gpu && !parakeet ? 'GPU' : 'CPU'}</span>
      <span class="spacer"></span>
      <span class="keys">
        {#each [['dictation_hold', t('dictation.hold')], ['dictation_paste_last', t('dictation.pasteLast')]] as [action, label] (action)}
          {@const key = shortcut(action!)}
          {#if key}<span class="key"><small>{label}</small><kbd>{key}</kbd></span>{/if}
        {/each}
      </span>
    </div>

    <div class="history">
      <div class="head">
        <span class="hud-label">{t('dictation.test.history')}</span>
        <small>{t('dictation.test.historyHint')}</small>
        <span class="spacer"></span>
        <button class="round" aria-label={t('dictation.test.clear')} title={t('dictation.test.clear')} disabled={dictation.history.length === 0} onclick={() => daemon.setDictation(daemon.client.call('dictation.history_clear'))}><Trash2 size={13} strokeWidth={1.5} /></button>
      </div>
      {#if dictation.history.length === 0}
        <p class="empty">{prefs.keep_history ? t('quick.dictationEmpty') : t('dictation.test.historyOff')}</p>
      {:else}
        <ul>
          {#each dictation.history as entry (entry.id)}
            <li>
              <span class="time">{formatClock(entry.at_unix_ms)}</span>
              <span class="text" title={entry.text}>{entry.text}</span>
              <span class="meta">{language(entry)} · {formatSeconds(entry.elapsed_ms)}</span>
              <button class="round" aria-label={t('dictation.test.copy')} title={copied === entry.id ? t('dictation.test.copied') : t('dictation.test.copy')} onclick={() => copy(entry)}><Copy size={13} strokeWidth={1.5} /></button>
              <button class="round" aria-label={t('dictation.test.reinsert')} title={t('dictation.test.reinsert')} onclick={() => daemon.run(daemon.client.call('dictation.reinsert', { id: entry.id }))}><CornerDownLeft size={13} strokeWidth={1.5} /></button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </section>
{/if}

<style>
  .dictation {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 16px;
    padding-top: 16px;
    border-top: 1px solid var(--line);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 12px;
  }

  .languages {
    display: flex;
    gap: 6px;
  }

  .chip {
    padding: 5px 12px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: var(--bg-sunken);
    font-size: 12px;
    color: var(--muted);
  }

  .chip.active {
    border-color: var(--accent);
    color: var(--ink-strong);
    background: color-mix(in srgb, var(--accent) 18%, var(--bg-sunken));
  }

  .chip:disabled {
    opacity: 0.4;
  }

  .spacer {
    flex: 1;
  }

  .keys {
    display: flex;
    gap: 12px;
  }

  .key {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .key small,
  .head small,
  .meta,
  .time {
    font-size: 11px;
    color: var(--faint);
    white-space: nowrap;
  }

  kbd {
    padding: 2px 6px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--bg-sunken);
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink);
  }

  .history {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 196px;
    overflow-y: auto;
  }

  li {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 36px;
    border-top: 1px solid var(--line);
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    color: var(--ink);
  }

  .time {
    font-family: var(--font-mono);
  }

  .empty {
    padding: 12px 0;
    font-size: 13px;
    color: var(--muted);
  }

  .round {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--ink);
  }

  .round:hover:not(:disabled) {
    border-color: var(--line-strong);
  }

  .round:disabled {
    opacity: 0.4;
  }

  :global(.sketch) .chip,
  :global(.sketch) .round,
  :global(.sketch) kbd {
    border-radius: 0;
  }
</style>
