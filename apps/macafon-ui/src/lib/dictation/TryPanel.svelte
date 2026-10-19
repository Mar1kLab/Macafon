<script lang="ts">
  // TRY IT (PRD §9.9.1), pinned at the bottom like TEST on the Microphone screen: hold the
  // button or Space to dictate through the whole pipeline; the text shows here instead of being
  // inserted. History (STT-015) expands upward: the last results, in daemon memory only.
  import { slide } from 'svelte/transition';
  import { ChevronUp, Copy, CornerDownLeft, Mic, Trash2 } from '@lucide/svelte';
  import PopupLook from './PopupLook.svelte';
  import type { DictationResult, DictationState } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { formatClock, formatSeconds } from './format';
  import { languageName } from './languages';

  let { dictation }: { dictation: DictationState } = $props();

  const prefs = $derived(dictation.prefs);
  const result = $derived(daemon.dictationTest);
  const usable = $derived(prefs.enabled && prefs.model !== null);
  const listening = $derived(dictation.phase === 'listening');
  const transcribing = $derived(dictation.phase === 'transcribing');
  const busy = $derived(listening || transcribing);
  let holding = $state(false);
  let historyOpen = $state(false);
  let copied = $state<number | null>(null);

  function modelName(id: string): string {
    return dictation.models.find((model) => model.id === id)?.name ?? id;
  }

  function language(entry: DictationResult): string {
    return entry.language ? languageName(dictation, entry.language) : t('dictation.test.auto');
  }

  async function start() {
    if (!usable || holding || busy) return;
    holding = true;
    const ok = await daemon.setDictation(daemon.client.call('dictation.test_start'));
    if (!ok) holding = false;
  }

  function stop() {
    if (!holding) return;
    holding = false;
    void daemon.setDictation(daemon.client.call('dictation.test_stop'));
  }

  async function copy(entry: DictationResult) {
    await navigator.clipboard.writeText(entry.text);
    copied = entry.id;
    setTimeout(() => {
      if (copied === entry.id) copied = null;
    }, 1500);
  }
</script>

<div class="panel">
  {#if historyOpen}
    <div class="history" transition:slide={{ duration: 300 }}>
      <div class="history-head">
        <span class="hud-label">{t('dictation.test.history')}</span>
        <span class="hint">{t('dictation.test.historyHint')}</span>
        <span class="spacer"></span>
        <button class="btn small ghost" disabled={dictation.history.length === 0} onclick={() => daemon.setDictation(daemon.client.call('dictation.history_clear'))}><Trash2 size={12} strokeWidth={1.5} />{t('dictation.test.clear')}</button>
      </div>
      {#if dictation.history.length === 0}
        <p class="muted">{prefs.keep_history ? t('dictation.test.historyEmpty') : t('dictation.test.historyOff')}</p>
      {:else}
        <ul>
          {#each dictation.history as entry (entry.id)}
            <li>
              <span class="time">{formatClock(entry.at_unix_ms)}</span>
              <span class="text" title={entry.text}>{entry.text}</span>
              <span class="badge">{language(entry)}</span>
              <span class="time">{formatSeconds(entry.elapsed_ms)}</span>
              <button class="btn icon ghost" aria-label={t('dictation.test.copy')} title={copied === entry.id ? t('dictation.test.copied') : t('dictation.test.copy')} onclick={() => copy(entry)}><Copy size={13} strokeWidth={1.5} /></button>
              <button class="btn icon ghost" aria-label={t('dictation.test.reinsert')} title={t('dictation.test.reinsert')} onclick={() => daemon.run(daemon.client.call('dictation.reinsert', { id: entry.id }))}><CornerDownLeft size={13} strokeWidth={1.5} /></button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}

  <div class="bar">
    <span class="hud-label label">{t('dictation.test.title')}</span>
    <button
      class="btn hold"
      class:on={holding || listening}
      disabled={!usable || (busy && !holding)}
      aria-pressed={holding}
      title={usable ? t('dictation.test.holdHint') : t('dictation.test.unavailable')}
      onpointerdown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId);
        void start();
      }}
      onpointerup={stop}
      onpointercancel={stop}
      onkeydown={(event) => {
        if (event.key !== ' ' || event.repeat) return;
        event.preventDefault();
        void start();
      }}
      onkeyup={(event) => {
        if (event.key !== ' ') return;
        event.preventDefault();
        stop();
      }}
    ><Mic size={13} strokeWidth={1.5} />{t('dictation.test.hold')}</button>

    <div class="output" aria-live="polite">
      {#if busy}
        <span class="look" class:analog={prefs.popup_look === 'analog'}>
          <PopupLook
            look={prefs.popup_look}
            source="live"
            {transcribing}
          />
        </span>
        <span class="muted">{listening ? t('dictation.test.listening') : t('dictation.test.transcribing')}</span>
      {:else if result}
        <span class="text" title={result.text}>“{result.text}”</span>
        <span class="meta">{formatSeconds(result.elapsed_ms)} · {modelName(result.model)} · {language(result)}</span>
        <button class="btn small" onclick={() => copy(result)}><Copy size={12} strokeWidth={1.5} />{copied === result.id ? t('dictation.test.copied') : t('dictation.test.copy')}</button>
      {:else}
        <span class="muted">{usable ? t('dictation.test.empty') : t('dictation.test.unavailable')}</span>
      {/if}
    </div>

    <button class="btn ghost history-toggle" aria-expanded={historyOpen} title={t('dictation.test.historyHint')} onclick={() => (historyOpen = !historyOpen)}>
      {t('dictation.test.history')}
      {#if dictation.history.length}<span class="count">{dictation.history.length}</span>{/if}
      <span class="chevron" class:open={historyOpen}><ChevronUp size={13} strokeWidth={1.5} /></span>
    </button>
  </div>
</div>


<style>
  .panel {
    flex: none;
    border-top: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    min-height: 64px;
    padding: 10px 24px;
  }

  .label {
    color: var(--ink);
  }

  .hold {
    min-width: 132px;
    touch-action: none;
  }

  .output {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .look {
    flex: none;
    width: 116px;
  }

  .look.analog {
    width: 96px;
  }

  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 14px;
    color: var(--ink-strong);
  }

  .meta,
  .time {
    flex: none;
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
    white-space: nowrap;
  }

  .muted {
    font-size: 13px;
    color: var(--muted);
  }

  .history-toggle {
    gap: 6px;
  }

  .count {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--accent-ink);
  }

  .chevron {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }

  .chevron.open {
    transform: rotate(180deg);
  }

  .history {
    max-height: 260px;
    overflow-y: auto;
    padding: 12px 24px 4px;
    border-bottom: 1px solid var(--line);
  }

  .history-head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 8px;
  }

  .spacer {
    flex: 1;
  }

  .hint {
    font-size: 12px;
    color: var(--faint);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 34px;
    border-top: 1px solid var(--line);
  }

  li .text {
    flex: 1;
    font-size: 13px;
    color: var(--ink);
  }
</style>
