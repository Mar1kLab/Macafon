<script lang="ts" module>
  /** Shown once, the first time the Dictation screen opens; the guide button opens it again. */
  const KEY = 'macafon.dictation.onboarded';
  export const dictationOnboarded = () => localStorage.getItem(KEY) === '1';
  export const markDictationOnboarded = () => localStorage.setItem(KEY, '1');
</script>

<script lang="ts">
  // Dictation's first-run guide (§8.4): step by step, what it is and how to start — first of all
  // that everything runs on this computer: no account, no cloud, audio and text never leave it.
  import { reveal } from '../motion';
  import { Download, HardDrive, Keyboard, Languages, Mic, ShieldCheck, WifiOff } from '@lucide/svelte';
  import type { Component } from 'svelte';
  import { fly } from 'svelte/transition';
  import Dialog from '../components/Dialog.svelte';
  import { displayShortcut } from '../settings/shortcut';
  import { languageName } from './languages';
  import { daemon } from '../state';
  import { t } from '../i18n';

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  type StepId = 'local' | 'model' | 'talk' | 'languages';
  const STEPS: { id: StepId; icon: Component }[] = [
    { id: 'local', icon: ShieldCheck },
    { id: 'model', icon: Download },
    { id: 'talk', icon: Keyboard },
    { id: 'languages', icon: Languages },
  ];
  let step = $state(0);
  let direction = $state(1);
  const current = $derived(STEPS[step]!);

  const hold = $derived(daemon.hotkeys?.bindings.find((binding) => binding.action === 'dictation_hold')?.shortcut ?? null);
  const layouts = $derived(daemon.dictation?.layout_languages ?? []);
  const hasModel = $derived(daemon.dictation?.models.some((model) => model.status === 'ready') ?? false);

  function go(next: number) {
    direction = next > step ? 1 : -1;
    step = next;
  }

  function finish() {
    markDictationOnboarded();
    step = 0;
    onclose();
  }
</script>

<Dialog {open} title={t('dictation.onboarding.title')} onclose={finish} width="600px">
  <div class="guide">
    <ol class="steps" aria-label={t('dictation.onboarding.steps')}>
      {#each STEPS as item, index (item.id)}
        <li>
          <button class:active={index === step} class:done={index < step} aria-current={index === step ? 'step' : undefined} onclick={() => go(index)}>
            <span class="n">{String(index + 1).padStart(2, '0')}</span>{t(`dictation.onboarding.${item.id}.tab`)}
          </button>
        </li>
      {/each}
    </ol>

    {#key step}
      <section class="step" in:fly={{ x: 24 * direction, ...reveal() }} aria-live="polite">
        <span class="icon"><current.icon size={28} strokeWidth={1.25} /></span>
        <h3>{t(`dictation.onboarding.${current.id}.title`)}</h3>
        <p>{t(`dictation.onboarding.${current.id}.body`)}</p>

        {#if current.id === 'local'}
          <ul class="facts">
            <li><HardDrive size={14} strokeWidth={1.5} />{t('dictation.onboarding.local.onDevice')}</li>
            <li><WifiOff size={14} strokeWidth={1.5} />{t('dictation.onboarding.local.offline')}</li>
            <li><ShieldCheck size={14} strokeWidth={1.5} />{t('dictation.onboarding.local.noAccount')}</li>
          </ul>
        {:else if current.id === 'model'}
          <ul class="facts">
            <li><span class="tag">Whisper</span>{t('dictation.onboarding.model.whisper')}</li>
            <li><span class="tag">Parakeet</span>{t('dictation.onboarding.model.parakeet')}</li>
          </ul>
          <p class="note">{hasModel ? t('dictation.onboarding.model.ready') : t('dictation.onboarding.model.where')}</p>
        {:else if current.id === 'talk'}
          <div class="keys">
            <kbd class:empty={!hold}>{hold ? displayShortcut(hold) : t('settings.hotkeys.notSet')}</kbd>
            <span><Mic size={14} strokeWidth={1.5} />{t('dictation.onboarding.talk.speak')}</span>
            <span>{t('dictation.onboarding.talk.release')}</span>
          </div>
          <p class="note">{t('dictation.onboarding.talk.esc')}</p>
        {:else}
          {#if layouts.length}
            <ul class="languages" aria-label={t('dictation.onboarding.languages.found')}>
              {#each layouts as language (language.code)}<li>{language.name}</li>{/each}
            </ul>
          {:else}
            <p class="note">{t('dictation.onboarding.languages.none', { name: languageName(daemon.dictation, 'en') })}</p>
          {/if}
          <p class="note">{t('dictation.onboarding.languages.more')}</p>
        {/if}
      </section>
    {/key}
  </div>

  {#snippet actions()}
    <button class="btn ghost skip" onclick={finish}>{t('dictation.onboarding.skip')}</button>
    {#if step > 0}<button class="btn" onclick={() => go(step - 1)}>{t('dictation.onboarding.back')}</button>{/if}
    {#if step < STEPS.length - 1}
      <button class="btn invert" onclick={() => go(step + 1)}>{t('dictation.onboarding.next')}</button>
    {:else}
      <button class="btn invert" onclick={finish}>{t('dictation.onboarding.done')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .guide {
    display: grid;
    gap: var(--space-5);
  }

  .steps {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--space-1);
  }

  .steps button {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: 8px 0 6px;
    border: 0;
    border-top: 2px solid var(--line);
    background: none;
    color: var(--faint);
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }

  .steps button.done {
    border-top-color: var(--accent);
    color: var(--muted);
  }

  .steps button.active {
    border-top-color: var(--accent);
    color: var(--ink-strong);
  }

  .steps button:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 2px;
  }

  .n {
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    letter-spacing: 0.1em;
    color: var(--accent-ink);
  }

  .step {
    display: grid;
    gap: var(--space-3);
    min-height: 260px;
    align-content: start;
  }

  .icon {
    width: 56px;
    height: 56px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--accent);
    color: var(--accent-ink);
  }

  h3 {
    font-family: var(--font-display);
    font-size: var(--text-xl);
    letter-spacing: -0.02em;
    color: var(--ink-strong);
  }

  p {
    font-size: var(--text-md);
    line-height: 1.5;
    color: var(--ink);
  }

  .note {
    font-size: var(--text-md);
    color: var(--muted);
  }

  .facts {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: var(--space-2);
  }

  .facts li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 8px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    font-size: var(--text-md);
    color: var(--ink);
  }

  .facts :global(svg) {
    flex: none;
    color: var(--app-teal-ink);
  }

  .tag {
    flex: none;
    width: 72px;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--accent-ink);
  }

  .keys {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-3);
    font-size: var(--text-md);
    color: var(--muted);
  }

  .keys span {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .keys span::before {
    content: '→';
    color: var(--faint);
  }

  kbd {
    padding: 6px 12px;
    border: 1px solid var(--line-strong);
    font-family: var(--font-mono);
    font-size: var(--text-md);
    color: var(--ink-strong);
  }

  kbd.empty {
    color: var(--faint);
  }

  .languages {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .languages li {
    padding: 4px 10px;
    border: 1px solid var(--accent);
    font-size: var(--text-md);
    color: var(--ink-strong);
  }

  .skip {
    margin-right: auto;
  }
</style>
