<script lang="ts">
  // Dictation in the hotkey overlay's right panel (§9.9.2): the language mode with its fixed
  // language, the engine and device, and the history with copy and insert-again — what is
  // wanted mid-conversation without opening the app.
  import { Copy, CornerDownLeft, Trash2 } from '@lucide/svelte';
  import { radioKeys } from '../components/radioKeys';
  import { formatClock, formatSeconds } from '../dictation/format';
  import { displayShortcut } from '../settings/shortcut';
  import { languageName, layoutLanguageOptions } from '../dictation/languages';
  import type { DictationEngine, DictationLanguagePolicy, DictationResult } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';

  const dictation = $derived(daemon.dictation);
  const prefs = $derived(dictation?.prefs);
  const parakeet = $derived(prefs?.engine === 'parakeet');
  const model = $derived(dictation?.models.find((m) => m.id === prefs?.model)?.name ?? '—');
  const hold = $derived(daemon.hotkeys?.bindings.find((binding) => binding.action === 'dictation_hold')?.shortcut ?? null);
  let copied = $state<number | null>(null);

  const policies: { value: DictationLanguagePolicy; label: string }[] = [
    { value: 'layout', label: t('dictation.language.layout') },
    { value: 'fixed', label: t('dictation.language.fixed') },
    { value: 'auto', label: t('dictation.language.auto') },
  ];
  /** The user's keyboard layout languages; others are picked on the Dictation screen. */
  const languages = $derived(layoutLanguageOptions(dictation ?? null));
  const engines: { value: DictationEngine; label: string }[] = [
    { value: 'whisper', label: t('dictation.engine.whisper') },
    { value: 'parakeet', label: t('dictation.engine.parakeet') },
  ];
  const engineReady = (engine: DictationEngine) => dictation?.models.some((m) => m.engine === engine && m.status === 'ready') ?? false;

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

{#snippet chips<T>(label: string, options: { value: T; label: string; disabled?: boolean }[], value: T, onchange: (value: T) => void, disabled = false)}
  <div class="row">
    <span class="hud-label">{label}</span>
    <div class="chips" role="radiogroup" aria-label={label} tabindex="-1" use:radioKeys>
      {#each options as option (option.label)}
        <button class="chip" class:active={value === option.value} role="radio" aria-checked={value === option.value} tabindex={value === option.value ? 0 : -1} disabled={disabled || option.disabled} onclick={() => onchange(option.value)}>{option.label}</button>
      {/each}
    </div>
  </div>
{/snippet}

{#if dictation && prefs}
  <section class="dictation" id="overlay-dictation" aria-label={t('quick.dictation')}>
    <header>
      <h2 class="hud-label">{t('quick.dictation')}</h2>
      <span class="badge">{model.toUpperCase()} · {prefs.gpu && !parakeet ? 'GPU' : 'CPU'}</span>
      {#if hold}<span class="hint">{t('dictation.hold')} <kbd>{displayShortcut(hold)}</kbd></span>{/if}
    </header>

    <div class="settings">
      {@render chips(t('dictation.language.label'), policies, parakeet ? 'auto' : prefs.language, (language) => daemon.setDictationPrefs({ language }), parakeet)}
      {@render chips(t('dictation.language.fixedLanguage'), languages, prefs.fixed_language, (fixed_language) => daemon.setDictationPrefs({ fixed_language, language: 'fixed' }), parakeet)}
      {@render chips(t('dictation.engine.label'), engines.map((engine) => ({ ...engine, disabled: !engineReady(engine.value) })), prefs.engine, (engine) => daemon.setDictationPrefs({ engine }))}
      {@render chips(t('dictation.performance.device'), [{ value: true, label: t('dictation.performance.gpu') }, { value: false, label: t('dictation.performance.cpu') }], parakeet ? false : prefs.gpu, (gpu) => daemon.setDictationPrefs({ gpu }), parakeet)}
    </div>

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
  .dictation {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    flex: 1;
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  h2 {
    color: var(--ink);
  }

  .hint {
    margin-left: auto;
    font-size: 11px;
    color: var(--faint);
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

  .settings {
    display: grid;
    gap: 8px;
  }

  .row {
    display: grid;
    grid-template-columns: 112px minmax(0, 1fr);
    align-items: center;
    gap: 8px;
  }

  .row .hud-label {
    font-size: 10px;
  }

  .chips {
    display: flex;
    gap: 4px;
  }

  .chip {
    padding: 4px 10px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: transparent;
    font-size: 12px;
    color: var(--muted);
  }

  .chip:hover:not(:disabled) {
    border-color: var(--line-strong);
    color: var(--ink);
  }

  .chip.active {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--ink-strong);
  }

  .chip:disabled {
    opacity: 0.35;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
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
