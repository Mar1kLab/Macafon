<script lang="ts">
  // The Displays first run (§9.10): profiles suggested from the connected monitors, created in
  // one click; then shortcuts for them, the way back, and a safe try.
  import { onMount } from 'svelte';
  import { Check, MonitorSmartphone } from '@lucide/svelte';
  import LayoutSketch from './LayoutSketch.svelte';
  import ProfileIcon from './ProfileIcon.svelte';
  import { profileSketch } from './profiles';
  import type { DisplayProfile } from '../ipc/protocol';
  import { displayShortcut } from '../settings/shortcut';
  import { daemon } from '../state';
  import { t } from '../i18n';

  let { onstep, onfinish, onskip }: { onstep: () => void; onfinish: () => void; onskip: () => void } = $props();

  let choices = $state<{ profile: DisplayProfile; checked: boolean; name: string }[]>([]);
  let step = $state<'pick' | 'shortcuts'>('pick');
  let created = $state<string[]>([]);
  let assign = $state(true);
  let busy = $state(false);

  const monitors = $derived(daemon.displays?.monitors ?? []);
  const shortcutOf = (action: string) => daemon.hotkeys?.bindings.find((binding) => binding.action === action)?.shortcut ?? null;
  const allOn = $derived(shortcutOf('display_all_on'));
  const revert = $derived(shortcutOf('display_revert'));
  const tryTarget = $derived(daemon.displays?.profiles.find((entry) => created.includes(entry.profile.id) && entry.profile.id !== daemon.displays?.active)?.profile ?? null);

  onMount(async () => {
    const suggestions = await daemon.run(daemon.client.call('displays.suggestions'));
    choices = (suggestions ?? []).map((profile) => ({ profile, checked: true, name: profile.name }));
  });

  async function create() {
    busy = true;
    // The screen keeps the first run while profiles appear, for the shortcuts step.
    onstep();
    const before = new Set(daemon.displays?.profiles.map((entry) => entry.profile.id));
    for (const choice of choices.filter((c) => c.checked && c.name.trim())) {
      await daemon.setDisplays(daemon.client.call('displays.create', { profile: { ...$state.snapshot(choice.profile), name: choice.name.trim() } }));
    }
    created = (daemon.displays?.profiles ?? []).map((entry) => entry.profile.id).filter((id) => !before.has(id));
    busy = false;
    if (created.length) step = 'shortcuts';
  }

  async function finish() {
    busy = true;
    if (assign) {
      for (const [index, id] of created.slice(0, 9).entries()) {
        if (shortcutOf(`display_profile.${id}`)) continue;
        const next = await daemon.run(daemon.client.call('hotkeys.set', { action: `display_profile.${id}`, shortcut: `Ctrl+Alt+Shift+${index + 1}`, secondary: false }));
        if (next) daemon.hotkeys = next;
      }
    }
    busy = false;
    onfinish();
  }
</script>

<section class="onboarding" aria-labelledby="displays-onboarding-title">
  <header>
    <span class="mark"><MonitorSmartphone size={22} strokeWidth={1.5} /></span>
    <div>
      <h2 id="displays-onboarding-title">{t('displays.onboarding.title')}</h2>
      <p>{t('displays.onboarding.body')}</p>
    </div>
  </header>

  {#if step === 'pick'}
    <div class="step">
      <span class="hud-label">01 · {t('displays.onboarding.pick')}</span>
      <p class="hint">{t('displays.onboarding.pickHint')}</p>
      <ul class="choices">
        {#each choices as choice (choice.profile.id)}
          <li class:off={!choice.checked}>
            <label class="check">
              <input type="checkbox" bind:checked={choice.checked} aria-label={choice.name} />
              <span class="box" aria-hidden="true">{#if choice.checked}<Check size={12} strokeWidth={2} />{/if}</span>
            </label>
            <span class="icon"><ProfileIcon icon={choice.profile.icon} size={16} /></span>
            <input class="input" bind:value={choice.name} maxlength="40" aria-label={t('displays.name')} disabled={!choice.checked} />
            <div class="sketch"><LayoutSketch monitors={profileSketch(choice.profile, monitors)} height={56} /></div>
          </li>
        {/each}
      </ul>
      <div class="actions">
        <button class="btn ghost" onclick={onskip}>{t('displays.onboarding.skip')}</button>
        <button class="btn invert" disabled={busy || !choices.some((c) => c.checked && c.name.trim())} onclick={create}>{t('displays.onboarding.create')}</button>
      </div>
    </div>
  {:else}
    <div class="step">
      <span class="hud-label">02 · {t('displays.onboarding.shortcutsTitle')}</span>
      <label class="assign">
        <input type="checkbox" bind:checked={assign} />
        <span class="box" aria-hidden="true">{#if assign}<Check size={12} strokeWidth={2} />{/if}</span>
        {t('displays.onboarding.shortcuts')}
      </label>
      <p class="safety">
        {t('displays.onboarding.safety', {
          allOn: `${t('displays.allOn')}${allOn ? ` (${displayShortcut(allOn)})` : ''}`,
          revert: `${t('displays.revert')}${revert ? ` (${displayShortcut(revert)})` : ''}`,
        })}
      </p>
      {#if tryTarget}
        <div class="try">
          <button class="btn" disabled={busy || daemon.displays?.pending !== null} onclick={() => daemon.applyDisplayProfile(tryTarget.id)}>{t('displays.onboarding.try')}</button>
          <span class="hint">{t('displays.onboarding.tryHint', { name: tryTarget.name })}</span>
        </div>
      {/if}
      <div class="actions">
        <button class="btn invert" disabled={busy} onclick={finish}>{t('displays.onboarding.finish')}</button>
      </div>
    </div>
  {/if}
</section>

<style>
  .onboarding { border: 1px solid var(--line); background: var(--bg-raised); padding: 24px; display: flex; flex-direction: column; gap: 24px; max-width: 760px; }
  header { display: flex; gap: 16px; align-items: flex-start; }
  .mark { width: 44px; height: 44px; flex: none; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--accent); color: var(--accent-ink); }
  h2 { margin: 0 0 6px; font-family: var(--font-display); font-size: 20px; letter-spacing: -0.02em; color: var(--ink-strong); }
  p { margin: 0; font-size: 13px; line-height: 1.5; color: var(--muted); }
  .step { display: flex; flex-direction: column; gap: 12px; }
  .hint { font-size: 12px; color: var(--muted); }
  .choices { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; border-top: 1px solid var(--line); }
  .choices li { display: grid; grid-template-columns: auto auto minmax(140px, 220px) 1fr; align-items: center; gap: 12px; padding: 8px 0; border-bottom: 1px solid var(--line); }
  .choices li.off .sketch, .choices li.off .icon { opacity: 0.4; }
  .icon { display: inline-flex; color: var(--muted); }
  .sketch { min-width: 0; }
  .input { height: 30px; padding: 0 8px; border: 1px solid var(--line); background: var(--bg-sunken); color: var(--ink); font-size: 13px; user-select: text; min-width: 0; }
  .input:focus { outline: none; border-color: var(--accent); }
  .input:disabled { color: var(--faint); }
  .check, .assign { display: inline-flex; align-items: center; gap: 10px; cursor: pointer; position: relative; font-size: 13px; color: var(--ink); }
  .check input, .assign input { position: absolute; opacity: 0; width: 1px; height: 1px; }
  .box { width: 16px; height: 16px; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--line-strong); color: var(--ink-strong); background: var(--bg-sunken); }
  input:checked + .box { background: var(--accent); border-color: var(--accent); }
  input:focus-visible + .box { outline: 1px solid var(--ink); outline-offset: 2px; }
  .safety { font-size: 12px; }
  .try { display: flex; align-items: center; gap: 12px; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
