<script lang="ts">
  // 04 / DICTATION (PRD §8.4, §9.9): local speech to text. The header turns it on (STT-001) and
  // shows the worker; the cards set the engine and models, language, input, output, the worker's
  // footprint, the popup and the vocabulary; TRY IT at the bottom runs the whole pipeline.
  import { TriangleAlert, X } from '@lucide/svelte';
  import Toggle from '../lib/components/Toggle.svelte';
  import Segmented from '../lib/components/Segmented.svelte';
  import RulerSlider from '../lib/components/RulerSlider.svelte';
  import LevelMeter from '../lib/components/LevelMeter.svelte';
  import PictureChoice from '../lib/settings/PictureChoice.svelte';
  import ShortcutKey from '../lib/soundpad/ShortcutKey.svelte';
  import Card from '../lib/dictation/Card.svelte';
  import EngineCard from '../lib/dictation/EngineCard.svelte';
  import PopupLook from '../lib/dictation/PopupLook.svelte';
  import TryPanel from '../lib/dictation/TryPanel.svelte';
  import { formatBytes } from '../lib/dictation/format';
  import { OSD_MAX_OFFSET_PCT } from '../lib/ipc/dsp';
  import type {
    DictationGamePolicy,
    DictationInsert,
    DictationLanguage,
    DictationLanguagePolicy,
    DictationPopupLook,
    DictationPopupPosition,
    DictationTap,
  } from '../lib/ipc/protocol';
  import { daemon } from '../lib/state';
  import { t } from '../lib/i18n';

  const MAX_WORDS = 50;

  const dictation = $derived(daemon.dictation);
  const prefs = $derived(dictation?.prefs);
  const hasModel = $derived(dictation?.models.some((model) => model.status === 'ready') ?? false);
  const parakeet = $derived(prefs?.engine === 'parakeet');
  let moreShortcuts = $state(false);
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
  const languages: { value: DictationLanguage; label: string }[] = [
    { value: 'en', label: t('dictation.language.en') },
    { value: 'uk', label: t('dictation.language.uk') },
    { value: 'ru', label: t('dictation.language.ru') },
  ];
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
  const unloads: { value: number; label: string }[] = [
    { value: 0, label: t('dictation.performance.now') },
    { value: 60, label: t('dictation.performance.minutes', { value: 1 }) },
    { value: 180, label: t('dictation.performance.minutes', { value: 3 }) },
    { value: 600, label: t('dictation.performance.minutes', { value: 10 }) },
    { value: -1, label: t('dictation.performance.never') },
  ];
  const looks: { value: DictationPopupLook; label: string; hint: string }[] = [
    { value: 'flow', label: t('dictation.popup.flow'), hint: t('dictation.popup.flowHint') },
    { value: 'analog', label: t('dictation.popup.analog'), hint: t('dictation.popup.analogHint') },
  ];
  const positions: { value: DictationPopupPosition; label: string }[] = [
    { value: 'bottom', label: t('dictation.popup.bottom') },
    { value: 'top', label: t('dictation.popup.top') },
  ];

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
      <span class="master" title={hasModel ? t('dictation.toggleHint') : t('dictation.test.unavailable')}>
        <span class="hud-label">{t('dictation.toggle')}</span>
        <Toggle checked={prefs.enabled} label={t('dictation.toggle')} disabled={!hasModel} onchange={(enabled) => daemon.setDictationPrefs({ enabled })} />
      </span>
    </header>

    {#if dictation.phase === 'error' && dictation.error}
      <p class="banner danger" role="alert"><TriangleAlert size={14} strokeWidth={1.5} />{t('dictation.errorBanner', { error: dictation.error })}</p>
    {/if}

    <div class="scroll">
      <section class="hero">
        <div class="intro">
          <h2>{t('dictation.title')} <em>{t('dictation.titleAccent')}</em>.</h2>
          <p>{t('dictation.subtitle')}</p>
        </div>
        <div class="keys">
          <div class="key"><span class="hud-label">{t('dictation.hold')}</span><ShortcutKey action="dictation_hold" label={t('settings.hotkeys.actions.dictation_hold')} /></div>
          <div class="key"><span class="hud-label">{t('dictation.handsFree')}</span><ShortcutKey action="dictation_toggle" label={t('settings.hotkeys.actions.dictation_toggle')} /></div>
          <div class="key"><span class="hud-label">{t('dictation.pasteLast')}</span><ShortcutKey action="dictation_paste_last" label={t('settings.hotkeys.actions.dictation_paste_last')} /></div>
          {#if moreShortcuts}
            <div class="key"><span class="hud-label">{t('dictation.copyLast')}</span><ShortcutKey action="dictation_copy_last" label={t('settings.hotkeys.actions.dictation_copy_last')} /></div>
            <div class="key"><span class="hud-label">{t('dictation.holdCopy')}</span><ShortcutKey action="dictation_hold_copy" label={t('settings.hotkeys.actions.dictation_hold_copy')} /></div>
          {/if}
          <div class="key-foot">
            <span class="hint">{t('dictation.escCancels')}</span>
            <button class="link hud-label" aria-expanded={moreShortcuts} onclick={() => (moreShortcuts = !moreShortcuts)}>{moreShortcuts ? t('dictation.lessShortcuts') : t('dictation.moreShortcuts')}</button>
          </div>
        </div>
      </section>

      <div class="grid">
        <EngineCard {dictation} />

        <Card number="02" title={t('dictation.language.title')} hint={t('dictation.language.hint')} color="violet" dimmed={parakeet} badge={parakeet ? t('dictation.language.autoOnly') : undefined}>
          <Segmented label={t('dictation.language.label')} options={policies} value={prefs.language} disabled={parakeet} onchange={(language) => daemon.setDictationPrefs({ language })} />
          <Segmented label={t('dictation.language.fixedLanguage')} options={languages} value={prefs.fixed_language} disabled={parakeet || prefs.language !== 'fixed'} onchange={(fixed_language) => daemon.setDictationPrefs({ fixed_language })} />
          <p class="body">{parakeet ? t('dictation.language.parakeetBody') : languageBody}</p>
        </Card>

        <Card number="03" title={t('dictation.input.title')} hint={t('dictation.input.hint')} color="lime">
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
        </Card>

        <Card number="04" title={t('dictation.output.title')} hint={t('dictation.output.hint')} color="pink">
          <Segmented label={t('dictation.output.insert')} options={inserts} value={prefs.insert} color="pink" hint={t('dictation.output.insertHint')} onchange={(insert) => daemon.setDictationPrefs({ insert })} />
          <Segmented label={t('dictation.output.games')} options={gamePolicies} value={prefs.fullscreen_games} color="pink" hint={t('dictation.output.gamesHint')} onchange={(fullscreen_games) => daemon.setDictationPrefs({ fullscreen_games })} />
          <label class="row">
            <span>{t('dictation.output.trailingSpace')}</span>
            <Toggle checked={prefs.trailing_space} label={t('dictation.output.trailingSpace')} color="pink" onchange={(trailing_space) => daemon.setDictationPrefs({ trailing_space })} />
          </label>
        </Card>

        <Card number="05" title={t('dictation.performance.title')} hint={t('dictation.performance.hint')} color="amber">
          <Segmented label={t('dictation.performance.unload')} options={unloads} value={prefs.unload_after_s} color="amber" hint={t('dictation.performance.unloadHint')} onchange={(unload_after_s) => daemon.setDictationPrefs({ unload_after_s })} />
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
          <label class="row" title={t('dictation.performance.gameModeHint')}>
            <span>{t('dictation.performance.gameMode')}</span>
            <Toggle checked={prefs.game_mode} label={t('dictation.performance.gameMode')} color="amber" onchange={(game_mode) => daemon.setDictationPrefs({ game_mode })} />
          </label>
          <label class="row" title={t('dictation.performance.preRollHint')}>
            <span>{t('dictation.performance.preRoll')}</span>
            <Toggle checked={prefs.pre_roll} label={t('dictation.performance.preRoll')} color="amber" onchange={(pre_roll) => daemon.setDictationPrefs({ pre_roll })} />
          </label>
          {#snippet footer()}
            {t('dictation.performance.worker')} ·
            {dictation.worker_ram_bytes
              ? t('dictation.performance.loaded', { ram: formatBytes(dictation.worker_ram_bytes), threads: dictation.worker_threads ?? '—' })
              : t('dictation.performance.unloaded')}
          {/snippet}
        </Card>

        <Card number="06" title={t('dictation.popup.title')} hint={t('dictation.popup.hint')} color="orange" class="popup">
          <PictureChoice label={t('dictation.popup.look')} options={looks} value={prefs.popup_look} onchange={(popup_look) => daemon.setDictationPrefs({ popup_look })}>
            {#snippet picture(look)}
              <span class="look-picture" class:analog={look === 'analog'}><span class="inner"><PopupLook {look} source={look === prefs.popup_look ? 'demo' : 'still'} /></span></span>
            {/snippet}
          </PictureChoice>
          <div class="popup-controls">
            <Segmented label={t('dictation.popup.position')} options={positions} value={prefs.popup_position} color="orange" onchange={(popup_position) => daemon.setDictationPrefs({ popup_position })} />
            <RulerSlider
              compact
              label={t('dictation.popup.offset')}
              value={prefs.popup_offset_pct}
              min={0}
              max={OSD_MAX_OFFSET_PCT}
              defaultValue={0}
              step={0.5}
              color="orange"
              format={(value) => `${value}%`}
              onchange={(popup_offset_pct) => daemon.setDictationPrefs({ popup_offset_pct })}
            />
          </div>
          <label class="row">
            <span>{t('dictation.popup.overlayTile')}</span>
            <Toggle checked={prefs.show_in_overlay} label={t('dictation.popup.overlayTile')} color="orange" onchange={(show_in_overlay) => daemon.setDictationPrefs({ show_in_overlay })} />
          </label>
        </Card>

        <Card number="07" title={t('dictation.vocabulary.title')} hint={t('dictation.vocabulary.hint')} color="teal" dimmed={parakeet} badge={parakeet ? t('dictation.language.autoOnly') : undefined}>
          <ul class="words" aria-label={t('dictation.vocabulary.title')}>
            {#each prefs.vocabulary as entry (entry)}
              <li>
                <span>{entry}</span>
                <button aria-label={t('dictation.vocabulary.remove', { word: entry })} title={t('dictation.vocabulary.remove', { word: entry })} disabled={parakeet} onclick={() => daemon.setDictationPrefs({ vocabulary: prefs.vocabulary.filter((w) => w !== entry) })}><X size={12} strokeWidth={1.5} /></button>
              </li>
            {/each}
          </ul>
          <input
            class="word"
            bind:value={word}
            placeholder={prefs.vocabulary.length >= MAX_WORDS ? t('dictation.vocabulary.full', { max: MAX_WORDS }) : t('dictation.vocabulary.placeholder')}
            aria-label={t('dictation.vocabulary.add')}
            maxlength={48}
            disabled={parakeet || prefs.vocabulary.length >= MAX_WORDS}
            onkeydown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                addWord();
              }
            }}
          />
          <p class="body">{parakeet ? t('dictation.vocabulary.parakeet') : t('dictation.vocabulary.hint')}</p>
        </Card>
      </div>
    </div>

    <TryPanel {dictation} />
  </div>
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
  }

  .hero {
    display: flex;
    align-items: flex-start;
    gap: 32px;
    margin-bottom: 24px;
  }

  .intro {
    flex: 1;
    min-width: 0;
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

  .intro p {
    margin-top: 6px;
    font-size: 14px;
    color: var(--muted);
  }

  .keys {
    display: grid;
    grid-template-columns: auto auto;
    align-items: center;
    gap: 6px 14px;
  }

  .key {
    display: contents;
  }

  .key .hud-label {
    justify-self: end;
  }

  .key-foot {
    grid-column: 1 / -1;
    display: flex;
    justify-content: flex-end;
    gap: 12px;
  }

  .hint {
    font-size: 12px;
    color: var(--faint);
  }

  .link {
    color: var(--accent-ink);
  }

  .link:hover {
    color: var(--ink);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 16px;
  }

  .grid :global(.engine) {
    grid-column: span 2;
  }

  .grid :global(.popup) {
    grid-column: span 2;
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

  .look-picture {
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 7;
    background: var(--bg);
    background-image:
      linear-gradient(var(--scanline) 1px, transparent 1px),
      linear-gradient(90deg, var(--scanline) 1px, transparent 1px);
    background-size: 12px 12px;
  }

  .look-picture .inner {
    width: 62%;
  }

  .look-picture.analog .inner {
    width: 54%;
  }

  .popup-controls {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: end;
    gap: 16px;
  }

  .words {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
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
