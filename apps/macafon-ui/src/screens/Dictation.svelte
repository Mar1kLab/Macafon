<script lang="ts">
  // 04 / DICTATION (PRD §8.4, §9.9): local speech to text. The header turns it on (STT-001) and
  // shows the worker; the cards set the engine and models, language, input, output, the worker's
  // footprint, the popup and the vocabulary; TRY IT at the bottom runs the whole pipeline.
  import { CircleHelp, Keyboard, Trash2, TriangleAlert, X } from '@lucide/svelte';
  import Dropdown from '../lib/components/Dropdown.svelte';
  import Onboarding, { dictationOnboarded } from '../lib/dictation/Onboarding.svelte';
  import { layoutLanguageOptions, otherLanguageOptions } from '../lib/dictation/languages';
  import Toggle from '../lib/components/Toggle.svelte';
  import Segmented from '../lib/components/Segmented.svelte';
  import RulerSlider from '../lib/components/RulerSlider.svelte';
  import LevelMeter from '../lib/components/LevelMeter.svelte';
  import { displayShortcut } from '../lib/settings/shortcut';
  import Card from '../lib/dictation/Card.svelte';
  import EngineCard from '../lib/dictation/EngineCard.svelte';
  import TryPanel from '../lib/dictation/TryPanel.svelte';
  import { formatBytes } from '../lib/dictation/format';
  import type {
    DictationGamePolicy,
    DictationInsert,
    DictationLanguagePolicy,
    DictationTap,
  } from '../lib/ipc/protocol';
  import { daemon } from '../lib/state';
  import { t } from '../lib/i18n';
  import SettingsLink from '../lib/components/SettingsLink.svelte';
  import { nav } from '../lib/state/nav.svelte';

  const MAX_WORDS = 50;


  const dictation = $derived(daemon.dictation);
  const prefs = $derived(dictation?.prefs);
  const hasModel = $derived(dictation?.models.some((model) => model.status === 'ready') ?? false);
  const parakeet = $derived(prefs?.engine === 'parakeet');
  let word = $state('');

  const phaseLabel = $derived.by(() => {
    if (!dictation) return '';
    const phase = dictation.phase;
    if (phase === 'ready' && dictation.loaded_model) {
      const model = dictation.models.find((m) => m.id === dictation.loaded_model);
      const ram = dictation.worker_ram_bytes ? ` · ${formatBytes(dictation.worker_ram_bytes)}` : '';
      return `${t('dictation.phase.ready')} · ${model?.name ?? dictation.loaded_model}${ram}`;
    }
    return t(`dictation.phase.${phase}`);
  });

  const policies: { value: DictationLanguagePolicy; label: string }[] = [
    { value: 'layout', label: t('dictation.language.layout') },
    { value: 'fixed', label: t('dictation.language.fixed') },
    { value: 'auto', label: t('dictation.language.auto') },
  ];
  /** The user's keyboard layout languages as chips; any other Whisper language from a list. */
  const languages = $derived(layoutLanguageOptions(dictation ?? null));
  const otherLanguages = $derived([{ value: '', label: t('dictation.language.other') }, ...otherLanguageOptions(dictation ?? null)]);
  /** First visit: the step-by-step guide. */
  let guide = $state(!dictationOnboarded());
  const taps: { value: DictationTap; label: string }[] = [
    { value: 'raw', label: t('dictation.input.raw') },
    { value: 'clean', label: t('dictation.input.clean') },
    { value: 'processed', label: t('dictation.input.processed') },
  ];
  const inserts: { value: DictationInsert; label: string }[] = [
    { value: 'paste', label: t('dictation.output.paste') },
    { value: 'type', label: t('dictation.output.type') },
  ];
  const gamePolicies: { value: DictationGamePolicy; label: string }[] = [
    { value: 'insert', label: t('dictation.output.gameInsert') },
    { value: 'copy', label: t('dictation.output.gameCopy') },
    { value: 'ignore', label: t('dictation.output.gameIgnore') },
  ];
  // STT-015: off, in memory until Macafon quits, or saved across restarts.
  type HistoryMode = 'off' | 'memory' | 'saved';
  const historyModes: { value: HistoryMode; label: string }[] = [
    { value: 'off', label: t('dictation.history.off') },
    { value: 'memory', label: t('dictation.history.memory') },
    { value: 'saved', label: t('dictation.history.saved') },
  ];
  const historyMode = $derived<HistoryMode>(!prefs?.keep_history ? 'off' : prefs.save_history ? 'saved' : 'memory');

  function setHistoryMode(mode: HistoryMode) {
    daemon.setDictationPrefs({ keep_history: mode !== 'off', save_history: mode === 'saved' });
  }

  const devices: { value: 'gpu' | 'cpu'; label: string }[] = [
    { value: 'gpu', label: t('dictation.performance.gpu') },
    { value: 'cpu', label: t('dictation.performance.cpu') },
  ];
  const unloads: { value: number; label: string }[] = [
    { value: 0, label: t('dictation.performance.now') },
    { value: 60, label: t('dictation.performance.minutes', { value: 1 }) },
    { value: 180, label: t('dictation.performance.minutes', { value: 3 }) },
    { value: 600, label: t('dictation.performance.minutes', { value: 10 }) },
    { value: -1, label: t('dictation.performance.never') },
  ];
  const shortcutRows = $derived(
    (
      [
        ['dictation_hold', t('dictation.hold')],
        ['dictation_toggle', t('dictation.handsFree')],
        ['dictation_paste_last', t('dictation.pasteLast')],
        ['dictation_copy_last', t('dictation.copyLast')],
        ['dictation_hold_copy', t('dictation.holdCopy')],
      ] as const
    ).map(([action, label]) => ({
      action,
      label,
      shortcut: daemon.hotkeys?.bindings.find((binding) => binding.action === action)?.shortcut ?? null,
    })),
  );

  /** Shortcuts and the popup live in Settings (§9.5); open it at that section. */
  const openSettings = (section: 'hotkeys' | 'dictation') => nav.settings(section);

  const languageBody = $derived(
    prefs?.language === 'fixed' ? t('dictation.language.fixedBody') : prefs?.language === 'auto' ? t('dictation.language.autoBody') : t('dictation.language.layoutBody'),
  );

  function addWord() {
    const value = word.trim();
    if (!prefs || !value || prefs.vocabulary.length >= MAX_WORDS) return;
    if (!prefs.vocabulary.some((existing) => existing.toLowerCase() === value.toLowerCase())) {
      daemon.setDictationPrefs({ vocabulary: [...prefs.vocabulary, value] });
    }
    word = '';
  }
</script>

{#if dictation && prefs}
  <div class="screen">
    <header class="top">
      <h1 class="hud-label">{t('dictation.section')}</h1>
      <span class="status" class:live={dictation.phase === 'listening' || dictation.phase === 'transcribing'} class:error={dictation.phase === 'error'} role="status">
        <span class="dot" aria-hidden="true"></span>{phaseLabel}
      </span>
      <span class="spacer"></span>
      {#each shortcutRows.slice(0, 2) as row (row.action)}
        <span class="key" title={t('dictation.escCancels')}>
          <span class="hud-label">{row.label}</span>
          <kbd class:empty={!row.shortcut}>{row.shortcut ? displayShortcut(row.shortcut) : t('settings.hotkeys.notSet')}</kbd>
        </span>
      {/each}
      <button class="btn icon ghost" aria-label={t('dictation.onboarding.open')} title={t('dictation.onboarding.open')} onclick={() => (guide = true)}><CircleHelp size={15} strokeWidth={1.5} /></button>
      <button class="btn icon ghost" aria-label={t('dictation.changeShortcuts')} title={t('dictation.changeShortcuts')} onclick={() => openSettings('hotkeys')}><Keyboard size={15} strokeWidth={1.5} /></button>
      <span class="master" title={hasModel ? t('dictation.toggleHint') : t('dictation.test.unavailable')}>
        <span class="hud-label">{t('dictation.toggle')}</span>
        <Toggle checked={prefs.enabled} label={t('dictation.toggle')} disabled={!hasModel} onchange={(enabled) => daemon.setDictationPrefs({ enabled })} />
      </span>
      <SettingsLink section="dictation" />
    </header>

    {#if dictation.phase === 'error' && dictation.error}
      <p class="banner danger" role="alert"><TriangleAlert size={14} strokeWidth={1.5} />{t('dictation.errorBanner', { error: dictation.error })}</p>
    {/if}

    <div class="scroll">
      {#if !hasModel}
        <!-- First run only: what dictation is, before the model is downloaded. -->
        <section class="hero">
          <h2>{t('dictation.title')} <em>{t('dictation.titleAccent')}</em>.</h2>
          <p>{t('dictation.subtitle')}</p>
        </section>
      {/if}

      <!-- Left to right, top to bottom: the model, how it understands you, the path from the
           microphone to the text and its history, then the resources it may use. -->
      <div class="grid">
        <EngineCard {dictation} />

        <div class="stack">
          <Card number="02" title={t('dictation.language.title')} hint={t('dictation.language.hint')} color="violet" dimmed={parakeet} badge={parakeet ? t('dictation.language.autoOnly') : undefined}>
            <Segmented wrap label={t('dictation.language.label')} options={policies} value={prefs.language} disabled={parakeet} hint={parakeet ? t('dictation.language.parakeetBody') : languageBody} onchange={(language) => daemon.setDictationPrefs({ language })} />
            {#if prefs.language === 'fixed' && !parakeet}
              <Segmented wrap label={t('dictation.language.fixedLanguage')} options={languages} value={prefs.fixed_language} onchange={(fixed_language) => daemon.setDictationPrefs({ fixed_language })} />
              {#if otherLanguages.length > 1}
                <Dropdown label={t('dictation.language.other')} options={otherLanguages} value={''} width="100%" onchange={(fixed_language) => fixed_language && daemon.setDictationPrefs({ fixed_language })} />
              {/if}
            {/if}
          </Card>

          <Card number="03" title={t('dictation.vocabulary.title')} hint={t('dictation.vocabulary.hint')} color="teal" dimmed={parakeet} badge={parakeet ? t('dictation.language.autoOnly') : undefined}>
            {#if prefs.vocabulary.length}
              <ul class="words" aria-label={t('dictation.vocabulary.title')}>
                {#each prefs.vocabulary as entry (entry)}
                  <li>
                    <span>{entry}</span>
                    <button aria-label={t('dictation.vocabulary.remove', { word: entry })} title={t('dictation.vocabulary.remove', { word: entry })} disabled={parakeet} onclick={() => daemon.setDictationPrefs({ vocabulary: prefs.vocabulary.filter((w) => w !== entry) })}><X size={12} strokeWidth={1.5} /></button>
                  </li>
                {/each}
              </ul>
            {/if}
            <input
              class="word"
              bind:value={word}
              placeholder={prefs.vocabulary.length >= MAX_WORDS ? t('dictation.vocabulary.full', { max: MAX_WORDS }) : t('dictation.vocabulary.placeholder')}
              aria-label={t('dictation.vocabulary.add')}
              title={parakeet ? t('dictation.vocabulary.parakeet') : t('dictation.vocabulary.hint')}
              maxlength={48}
              disabled={parakeet || prefs.vocabulary.length >= MAX_WORDS}
              onkeydown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  addWord();
                }
              }}
            />
          </Card>
        </div>

        <Card number="04" title={t('dictation.input.title')} hint={t('dictation.input.hint')} color="lime">
          <Segmented label={t('dictation.input.tap')} options={taps} value={prefs.tap} color="lime" hint={t('dictation.input.tapHint')} onchange={(tap) => daemon.setDictationPrefs({ tap })} />
          {#if prefs.tap === 'processed'}
            <LevelMeter label={t('dictation.input.level')} peakKey="output_peak_dbfs" rmsKey="output_rms_dbfs" color="lime" />
          {:else}
            <LevelMeter label={t('dictation.input.level')} peakKey="input_peak_dbfs" rmsKey="input_rms_dbfs" color="lime" />
          {/if}
          <label class="row" title={t('dictation.input.muteHint')}>
            <span>{t('dictation.input.mute')}</span>
            <Toggle checked={prefs.mute_mic_while_dictating} label={t('dictation.input.mute')} color="lime" onchange={(mute_mic_while_dictating) => daemon.setDictationPrefs({ mute_mic_while_dictating })} />
          </label>
          <label class="row" title={t('dictation.performance.preRollHint')}>
            <span>{t('dictation.performance.preRoll')}</span>
            <Toggle checked={prefs.pre_roll} label={t('dictation.performance.preRoll')} color="lime" onchange={(pre_roll) => daemon.setDictationPrefs({ pre_roll })} />
          </label>
        </Card>

        <Card number="05" title={t('dictation.output.title')} hint={t('dictation.output.hint')} color="pink">
          <Segmented label={t('dictation.output.insert')} options={inserts} value={prefs.insert} color="pink" hint={t('dictation.output.insertHint')} onchange={(insert) => daemon.setDictationPrefs({ insert })} />
          <Segmented label={t('dictation.output.games')} options={gamePolicies} value={prefs.fullscreen_games} color="pink" hint={t('dictation.output.gamesHint')} onchange={(fullscreen_games) => daemon.setDictationPrefs({ fullscreen_games })} />
          <label class="row">
            <span>{t('dictation.output.trailingSpace')}</span>
            <Toggle checked={prefs.trailing_space} label={t('dictation.output.trailingSpace')} color="pink" onchange={(trailing_space) => daemon.setDictationPrefs({ trailing_space })} />
          </label>
        </Card>

        <Card number="06" title={t('dictation.history.title')} hint={t('dictation.history.hint')} color="orange">
          <Segmented label={t('dictation.history.keep')} options={historyModes} value={historyMode} color="orange" onchange={setHistoryMode} />
          <p class="body">{t(`dictation.history.${historyMode}Body`)}</p>
          {#snippet footer()}
            <span class="history-foot">
              <span>{t('dictation.history.count', { count: dictation.history.length })}</span>
              <button class="link" disabled={dictation.history.length === 0} onclick={() => daemon.setDictation(daemon.client.call('dictation.history_clear'))}><Trash2 size={12} strokeWidth={1.5} />{t('dictation.test.clear')}</button>
            </span>
          {/snippet}
        </Card>

        <Card number="07" title={t('dictation.performance.title')} hint={t('dictation.performance.hint')} color="amber" class="performance">
          <div class="perf">
            <Segmented label={t('dictation.performance.device')} options={devices} value={parakeet ? 'cpu' : prefs.gpu ? 'gpu' : 'cpu'} color="amber" disabled={parakeet} hint={parakeet ? t('dictation.performance.deviceParakeet') : t('dictation.performance.deviceHint')} onchange={(device) => daemon.setDictationPrefs({ gpu: device === 'gpu' })} />
            <Segmented label={t('dictation.performance.unload')} options={unloads} value={prefs.unload_after_s} color="amber" hint={t('dictation.performance.unloadHint')} onchange={(unload_after_s) => daemon.setDictationPrefs({ unload_after_s })} />
            <label class="row" title={t('dictation.performance.gameModeHint')}>
              <span>{t('dictation.performance.gameMode')}</span>
              <Toggle checked={prefs.game_mode} label={t('dictation.performance.gameMode')} color="amber" onchange={(game_mode) => daemon.setDictationPrefs({ game_mode })} />
            </label>
            <RulerSlider
              compact
              label={t('dictation.performance.threads')}
              value={prefs.threads || dictation.default_threads}
              min={1}
              max={dictation.logical_cores}
              defaultValue={dictation.default_threads}
              step={1}
              color="amber"
              format={(value) => t('dictation.performance.threadsValue', { value, max: dictation.logical_cores })}
              onchange={(threads) => daemon.setDictationPrefs({ threads: threads === dictation.default_threads ? 0 : threads })}
            />
          </div>
          {#snippet footer()}
            {t('dictation.performance.worker')} ·
            {dictation.worker_ram_bytes
              ? t('dictation.performance.loaded', { ram: formatBytes(dictation.worker_ram_bytes), device: prefs.gpu && !parakeet ? 'GPU' : 'CPU', threads: dictation.worker_threads ?? '—' })
              : t('dictation.performance.unloaded')}
          {/snippet}
        </Card>
      </div>
    </div>

    <TryPanel {dictation} />
  </div>
  <Onboarding open={guide} onclose={() => (guide = false)} />
{:else}
  <div class="loading" role="status">{t('daemon.loading')}</div>
{/if}

<style>
  .screen {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 59px;
    padding: 14px 24px;
    border-bottom: 1px solid var(--line);
  }

  h1 {
    color: var(--ink);
    white-space: nowrap;
    margin-right: 8px;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 3px 8px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    font-family: var(--font-mono);
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .dot {
    width: 6px;
    height: 6px;
    background: var(--faint);
  }

  .status.live {
    color: var(--ink);
  }

  .status.live .dot {
    background: var(--danger);
    animation: blink 1s steps(1) infinite;
  }

  .status.error {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 50%, transparent);
  }

  .status.error .dot {
    background: var(--danger);
  }

  .spacer {
    flex: 1;
  }

  .master {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .master .hud-label {
    color: var(--ink);
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 24px;
    border-bottom: 1px solid var(--line);
    font-size: 13px;
  }

  .banner.danger {
    color: var(--danger);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px;
    container-type: inline-size;
  }

  .hero {
    margin-bottom: 24px;
  }

  h2 {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 32px;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
  }

  h2 em {
    font-family: var(--font-serif);
    font-weight: 400;
    font-style: italic;
  }

  .hero p {
    margin-top: 6px;
    font-size: 14px;
    color: var(--muted);
  }

  .key {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  kbd {
    padding: 3px 8px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    font-family: var(--font-mono);
    font-size: 12px;
    text-align: center;
    color: var(--ink-strong);
  }

  kbd.empty {
    color: var(--faint);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 16px;
  }

  .grid :global(.engine) {
    grid-column: span 2;
  }

  .stack {
    display: grid;
    gap: 16px;
    align-content: start;
  }

  .grid :global(.performance) {
    grid-column: span 3;
  }

  /* Narrow windows and big UI zoom: the model card takes the whole row, the rest go two by two,
     then one by one, so nothing is cut at the card edges. */
  @container (max-width: 1040px) {
    .grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .grid :global(.engine),
    .grid :global(.performance) {
      grid-column: 1 / -1;
    }
  }

  @container (max-width: 640px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .perf {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    align-items: center;
    gap: 12px 32px;
  }

  .history-foot {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .history-foot > span {
    flex: 1;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    color: var(--muted);
    text-transform: uppercase;
  }

  .link:hover:not(:disabled) {
    color: var(--ink);
  }

  .link:disabled {
    opacity: 0.4;
  }

  .body {
    font-size: 12.5px;
    color: var(--muted);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
    color: var(--ink);
  }





  .words {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    /* The defaults alone are dozens of words: a few rows, the rest scrolls. */
    max-height: 100px;
    overflow-y: auto;
    padding-right: 2px;
  }

  .words li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 4px 3px 8px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    font-size: 12.5px;
    color: var(--ink);
  }

  .words button {
    display: inline-flex;
    color: var(--faint);
  }

  .words button:hover:not(:disabled) {
    color: var(--ink);
  }

  .word {
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--ink);
    font: inherit;
    font-size: 13px;
  }

  .word:focus {
    outline: none;
    border-color: var(--accent);
  }

  .word::placeholder {
    color: var(--faint);
  }

  .loading {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--muted);
  }

  @keyframes blink {
    50% {
      opacity: 0.2;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .status.live .dot {
      animation: none;
    }
  }
</style>
