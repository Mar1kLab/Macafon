<script lang="ts">
  // Quick settings; saved dictations have their own panel.
  import { radioKeys } from '../components/radioKeys';
  import { formatBytes } from '../dictation/format';
  import { layoutLanguageOptions } from '../dictation/languages';
  import type { DictationEngine, DictationLanguagePolicy } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';

  const dictation = $derived(daemon.dictation);
  const prefs = $derived(dictation?.prefs);
  const parakeet = $derived(prefs?.engine === 'parakeet');
  const model = $derived(dictation?.models.find((m) => m.id === prefs?.model)?.name ?? '—');
  /** Whether the model sits in memory now (it loads on first use, unloads when idle). */
  const loaded = $derived(!!dictation?.loaded_model);
  const memory = $derived(
    loaded
      ? t('quick.dictationLoaded', { ram: dictation?.worker_ram_bytes ? formatBytes(dictation.worker_ram_bytes) : '—' })
      : t('quick.dictationUnloaded'),
  );

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
      <span class="memory" class:loaded role="status" {@attach tip(loaded ? t('quick.dictationLoadedHint') : t('quick.dictationUnloadedHint'))}><i aria-hidden="true"></i>{memory}</span>
    </header>

    <div class="settings">
      {@render chips(t('dictation.language.label'), policies, parakeet ? 'auto' : prefs.language, (language) => daemon.setDictationPrefs({ language }), parakeet)}
      {@render chips(t('dictation.language.fixedLanguage'), languages, prefs.fixed_language, (fixed_language) => daemon.setDictationPrefs({ fixed_language, language: 'fixed' }), parakeet)}
      {@render chips(t('dictation.engine.label'), engines.map((engine) => ({ ...engine, disabled: !engineReady(engine.value) })), prefs.engine, (engine) => daemon.setDictationPrefs({ engine }))}
      {@render chips(t('dictation.performance.device'), [{ value: true, label: t('dictation.performance.gpu') }, { value: false, label: t('dictation.performance.cpu') }], parakeet ? false : prefs.gpu, (gpu) => daemon.setDictationPrefs({ gpu }), parakeet)}
    </div>

  </section>
{/if}

<style>
  .dictation {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-height: 0;
    min-width: 0;
    flex: 1;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    flex: none;
  }

  h2 {
    color: var(--ink);
  }

  .memory {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--faint);
    transition: color var(--dur) var(--ease);
  }

  .memory i {
    width: 6px;
    height: 6px;
    border: 1px solid var(--faint);
    transition:
      background var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  .memory.loaded {
    color: var(--app-teal-ink);
  }

  .memory.loaded i {
    border-color: var(--app-teal-ink);
    background: var(--app-teal-ink);
  }

  .settings {
    display: grid;
    gap: var(--space-2);
    flex: none;
  }

  .row {
    display: grid;
    grid-template-columns: 112px minmax(0, 1fr);
    align-items: center;
    gap: var(--space-2);
  }

  .row .hud-label {
    font-size: var(--text-2xs);
  }

  .chips {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }

  .chip {
    padding: 4px 10px;
    border: 1px solid var(--line);
    border-radius: 999px;
    background: transparent;
    font-size: var(--text-sm);
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

  :global(.sketch) .chip {
    border-radius: 0;
  }

  .dictation { overflow-y: auto; overscroll-behavior: contain; }
  .chip:focus-visible { outline: 1px solid var(--accent); outline-offset: 2px; }
  :global(.body.down) .settings { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  :global(.body.down) .row { grid-template-columns: minmax(0, 1fr); }
</style>
