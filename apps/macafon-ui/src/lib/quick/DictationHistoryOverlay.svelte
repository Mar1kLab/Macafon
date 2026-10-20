<script lang="ts">
  import { Copy, CornerDownLeft, Trash2 } from '@lucide/svelte';
  import { formatClock, formatSeconds } from '../dictation/format';
  import { languageName } from '../dictation/languages';
  import type { DictationResult } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';
  const dictation = $derived(daemon.dictation);
  const prefs = $derived(dictation?.prefs);
  let copied = $state<number | null>(null);
  async function copy(entry: DictationResult) {
    await navigator.clipboard.writeText(entry.text);
    copied = entry.id;
    setTimeout(() => {
      if (copied === entry.id) copied = null;
    }, 1500);
  }

  function language(entry: DictationResult): string {
    return entry.language ? languageName(dictation ?? null, entry.language) : t('dictation.test.auto');
  }

</script>
{#if dictation && prefs}
  <section class="history" id="overlay-history" aria-label={t('settings.overlay.part.dictation_history')}>
    <div class="head">
      <span class="hud-label">{t('dictation.test.history')}</span>
      <span class="spacer"></span>
      <button class="round" aria-label={t('dictation.test.clear')} title={t('dictation.test.clear')} disabled={dictation.history.length === 0} onclick={() => daemon.setDictation(daemon.client.call('dictation.history_clear'))}><Trash2 size={13} strokeWidth={1.5} /></button>
    </div>
    {#if dictation.history.length === 0}
      <p class="empty">{prefs.keep_history ? t('quick.dictationEmpty') : t('dictation.test.historyOff')}</p>
    {:else}
      <ul>
        {#each dictation.history as entry (entry.id)}
          <li>
            <span class="text" title={entry.text}>{entry.text}</span>
            <span class="meta">{formatClock(entry.at_unix_ms)} · {language(entry)} · {formatSeconds(entry.elapsed_ms)}</span>
            <span class="actions">
              <button class="round" aria-label={t('dictation.test.copy')} title={copied === entry.id ? t('dictation.test.copied') : t('dictation.test.copy')} onclick={() => copy(entry)}><Copy size={13} strokeWidth={1.5} /></button>
              <button class="round" aria-label={t('dictation.test.reinsert')} title={t('dictation.test.reinsert')} onclick={() => daemon.run(daemon.client.call('dictation.reinsert', { id: entry.id }))}><CornerDownLeft size={13} strokeWidth={1.5} /></button>
            </span>
          </li>
        {/each}
      </ul>
    {/if}

  </section>
{/if}
<style>
  .history { display: flex; flex-direction: column; min-height: 0; min-width: 0; flex: 1; gap: 12px; }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: none;
  }

  .spacer {
    flex: 1;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
    scrollbar-gutter: stable;
    overscroll-behavior: contain;
  }

  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-rows: auto auto;
    column-gap: 10px;
    padding: 6px 0;
    border-top: 1px solid var(--line);
  }

  .text,
  .meta {
    grid-column: 1;
  }

  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    color: var(--ink);
  }

  .meta {
    font-size: 11px;
    color: var(--faint);
  }

  .actions {
    grid-column: 2;
    grid-row: 1 / 3;
    align-self: center;
    display: flex;
    gap: 4px;
  }

  .empty {
    padding: 8px 0;
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
    border-radius: 0;
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


  .round:focus-visible { outline: 1px solid var(--accent); outline-offset: 2px; }
</style>
