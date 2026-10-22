<script lang="ts">
  import { CornerDownLeft, Trash2 } from '@lucide/svelte';
  import Check from '../icons/animated/Check.svelte';
  import Copy from '../icons/animated/Copy.svelte';
  import { formatClock, formatSeconds } from '../dictation/format';
  import { Copier, clearHistory, reinsert, resultLanguage } from '../dictation/history.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';
  const dictation = $derived(daemon.dictation);
  const prefs = $derived(dictation?.prefs);
  const copier = new Copier();
</script>
{#if dictation && prefs}
  <section class="history" id="overlay-history" aria-label={t('settings.overlay.part.dictation_history')}>
    <div class="head">
      <span class="hud-label">{t('dictation.test.history')}</span>
      <span class="spacer"></span>
      <button class="round" aria-label={t('dictation.test.clear')} {@attach tip(t('dictation.test.clear'))} disabled={dictation.history.length === 0} onclick={clearHistory}><Trash2 size={13} strokeWidth={1.5} /></button>
    </div>
    {#if dictation.history.length === 0}
      <p class="empty">{prefs.keep_history ? t('quick.dictationEmpty') : t('dictation.test.historyOff')}</p>
    {:else}
      <ul>
        {#each dictation.history as entry (entry.id)}
          <li>
            <!-- Focusable so the keyboard can read a cut-off entry in its tooltip. -->
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <span class="text" tabindex="0" {@attach tip(entry.text)}>{entry.text}</span>
            <span class="meta">{formatClock(entry.at_unix_ms)} · {resultLanguage(dictation, entry)} · {formatSeconds(entry.elapsed_ms)}</span>
            <span class="actions">
              <button class="round" aria-label={t('dictation.test.copy')} {@attach tip(copier.id === entry.id ? t('dictation.test.copied') : t('dictation.test.copy'))} onclick={() => copier.copy(entry)}>{#if copier.id === entry.id}<Check size={13} animate />{:else}<Copy size={13} />{/if}</button>
              <button class="round" aria-label={t('dictation.test.reinsert')} {@attach tip(t('dictation.test.reinsert'))} onclick={() => reinsert(entry)}><CornerDownLeft size={13} strokeWidth={1.5} /></button>
            </span>
          </li>
        {/each}
      </ul>
    {/if}

  </section>
{/if}
<style>
  .history { display: flex; flex-direction: column; min-height: 0; min-width: 0; flex: 1; gap: var(--space-3); }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
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
    column-gap: var(--space-2);
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
    font-size: var(--text-md);
    color: var(--ink);
  }

  .meta {
    font-size: var(--text-xs);
    color: var(--faint);
  }

  .actions {
    grid-column: 2;
    grid-row: 1 / 3;
    align-self: center;
    display: flex;
    gap: var(--space-1);
  }

  .empty {
    padding: 8px 0;
    font-size: var(--text-md);
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
