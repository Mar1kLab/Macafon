<script lang="ts">
  // Displays (PRD §9.10): saved monitor layouts as cards with a to-scale sketch, the current
  // layout with "Save as profile", the DISP-004 confirmation and a first-run card.
  import { GripVertical, Pencil, RefreshCw, RotateCcw, Trash2, MonitorCheck } from '@lucide/svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import LayoutSketch from '../lib/displays/LayoutSketch.svelte';
  import ProfileIcon from '../lib/displays/ProfileIcon.svelte';
  import SaveDialog from '../lib/displays/SaveDialog.svelte';
  import EditDialog from '../lib/displays/EditDialog.svelte';
  import Onboarding from '../lib/displays/Onboarding.svelte';
  import { useCountdown } from '../lib/displays/countdown.svelte';
  import { currentSketch, profileSketch } from '../lib/displays/profiles';
  import type { DisplayProfile } from '../lib/ipc/protocol';
  import { displayShortcut } from '../lib/settings/shortcut';
  import { daemon } from '../lib/state';
  import { t } from '../lib/i18n';

  const ONBOARDING_KEY = 'macafon.displays.onboarding';

  const displays = $derived(daemon.displays);
  const pending = $derived(displays?.pending ?? null);
  const countdown = useCountdown(() => pending?.revert_at_unix_ms ?? null);
  const pendingName = $derived(pending?.profile ? (displays?.profiles.find((entry) => entry.profile.id === pending.profile)?.profile.name ?? '') : t('displays.allOn'));

  let skipped = $state(localStorage.getItem(ONBOARDING_KEY) === 'done');
  /** The first run keeps showing after it created profiles, for its shortcuts step. */
  let onboardingOpen = $state(false);
  const showOnboarding = $derived(onboardingOpen || (!skipped && (displays?.profiles.length ?? 0) === 0));

  let saving = $state(false);
  let editing = $state<DisplayProfile | null>(null);
  let deleting = $state<DisplayProfile | null>(null);
  let dragged = $state<string | null>(null);

  const shortcutOf = (id: string) => daemon.hotkeys?.bindings.find((binding) => binding.action === `display_profile.${id}`)?.shortcut ?? null;

  function closeOnboarding() {
    onboardingOpen = false;
    skipped = true;
    localStorage.setItem(ONBOARDING_KEY, 'done');
  }

  function move(id: string, to: number) {
    if (!displays) return;
    const ids = displays.profiles.map((entry) => entry.profile.id).filter((other) => other !== id);
    ids.splice(Math.max(0, Math.min(to, ids.length)), 0, id);
    void daemon.setDisplays(daemon.client.call('displays.reorder', { ids }));
  }

  /** Arrows on a card's handle move it one place. */
  function onkeydown(event: KeyboardEvent, id: string, index: number) {
    const step = event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : 0;
    if (!step) return;
    event.preventDefault();
    move(id, index + step);
  }

  async function recapture(profile: DisplayProfile) {
    if (await daemon.setDisplays(daemon.client.call('displays.recapture', { id: profile.id }))) {
      daemon.toast(t('displays.recaptured', { name: profile.name }));
    }
  }
</script>

{#if displays}
  <div class="screen">
    <header class="top">
      <h1 class="hud-label">{t('displays.title')}</h1>
      <span class="spacer"></span>
      {#if displays.can_revert && !pending}
        <button class="btn" onclick={() => daemon.setDisplays(daemon.client.call('displays.revert'))}><RotateCcw size={13} strokeWidth={1.5} />{t('displays.revert')}</button>
      {/if}
      <button class="btn" disabled={pending !== null} onclick={() => daemon.setDisplays(daemon.client.call('displays.all_on'))}><MonitorCheck size={13} strokeWidth={1.5} />{t('displays.allOn')}</button>
    </header>

    <div class="content">
      {#if pending}
        <div class="pending" role="alertdialog" aria-labelledby="displays-keep-title">
          <div class="text">
            <span id="displays-keep-title" class="question">{t('displays.keepTitle')}</span>
            <span class="detail">{pendingName} · <span class="count" aria-live="polite">{t('displays.reverting', { seconds: countdown.seconds })}</span></span>
          </div>
          <button class="btn" onclick={() => daemon.setDisplays(daemon.client.call('displays.confirm', { keep: false }))}>{t('displays.revertNow')}</button>
          <button class="btn invert" onclick={() => daemon.setDisplays(daemon.client.call('displays.confirm', { keep: true }))}>{t('displays.keep')}</button>
        </div>
      {/if}

      {#if showOnboarding}
        <Onboarding onstep={() => (onboardingOpen = true)} onfinish={closeOnboarding} onskip={closeOnboarding} />
      {:else}
        <section aria-labelledby="displays-profiles">
          <div class="section-head">
            <h2 id="displays-profiles" class="hud-label">{t('displays.profiles')}</h2>
            <span class="hint">{t('displays.hint')}</span>
          </div>
          {#if displays.profiles.length}
            <ul class="cards">
              {#each displays.profiles as { profile, missing }, index (profile.id)}
                {@const active = displays.active === profile.id}
                {@const shortcut = shortcutOf(profile.id)}
                <li
                  class="card"
                  class:active
                  class:dragging={dragged === profile.id}
                  draggable="true"
                  aria-label={profile.name}
                  ondragstart={(event) => {
                    dragged = profile.id;
                    event.dataTransfer?.setData('text/plain', profile.id);
                  }}
                  ondragend={() => (dragged = null)}
                  ondragover={(event) => dragged && event.preventDefault()}
                  ondrop={(event) => {
                    event.preventDefault();
                    if (dragged && dragged !== profile.id) move(dragged, index);
                    dragged = null;
                  }}
                >
                  <div class="card-head">
                    <span class="icon"><ProfileIcon icon={profile.icon} size={16} /></span>
                    <h3 class="name" title={profile.name}>{profile.name}</h3>
                    {#if active}<span class="badge">{t('displays.active')}</span>{/if}
                    <button class="btn icon ghost grip" aria-label={t('displays.reorder', { name: profile.name })} title={t('displays.reorderHint')} onkeydown={(event) => onkeydown(event, profile.id, index)}><GripVertical size={13} strokeWidth={1.5} /></button>
                  </div>
                  <LayoutSketch monitors={profileSketch(profile, displays.monitors)} height={84} />
                  {#if missing.length}<p class="warn">{t('displays.missing', { names: missing.join(', ') })}</p>{/if}
                  <div class="card-foot">
                    {#if shortcut}<kbd>{displayShortcut(shortcut)}</kbd>{:else}<span class="faint" title={t('displays.setShortcut')}>{t('displays.noShortcut')}</span>{/if}
                    <span class="spacer"></span>
                    <button class="btn icon ghost" aria-label={`${t('displays.edit')}: ${profile.name}`} title={t('displays.edit')} onclick={() => (editing = profile)}><Pencil size={13} strokeWidth={1.5} /></button>
                    <button class="btn icon ghost" aria-label={`${t('displays.recapture')}: ${profile.name}`} title={t('displays.recapture')} onclick={() => recapture(profile)}><RefreshCw size={13} strokeWidth={1.5} /></button>
                    <button class="btn icon ghost" aria-label={`${t('displays.delete')}: ${profile.name}`} title={t('displays.delete')} onclick={() => (deleting = profile)}><Trash2 size={13} strokeWidth={1.5} /></button>
                    <button class="btn" class:invert={!active} disabled={active || pending !== null} onclick={() => daemon.applyDisplayProfile(profile.id)}>{active ? t('displays.active') : t('displays.apply')}</button>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {/if}

      <section class="current" aria-labelledby="displays-current">
        <div class="section-head">
          <h2 id="displays-current" class="hud-label">{t('displays.current')}</h2>
          <span class="hint">{t('displays.currentHint')}</span>
          <span class="spacer"></span>
          <button class="btn invert" onclick={() => (saving = true)}>{t('displays.save')}</button>
        </div>
        <div class="current-sketch"><LayoutSketch monitors={currentSketch(displays.monitors)} height={150} /></div>
      </section>
    </div>
  </div>

  <SaveDialog open={saving} onclose={() => (saving = false)} />
  <EditDialog profile={editing} onclose={() => (editing = null)} />
  <Dialog open={deleting !== null} title={t('displays.deleteTitle', { name: deleting?.name ?? '' })} onclose={() => (deleting = null)}>
    <p class="body">{t('displays.deleteBody')}</p>
    {#snippet actions()}
      <button class="btn" onclick={() => (deleting = null)}>{t('common.cancel')}</button>
      <button
        class="btn danger on"
        onclick={async () => {
          if (deleting) await daemon.setDisplays(daemon.client.call('displays.delete', { id: deleting.id }));
          deleting = null;
        }}
      >{t('displays.delete')}</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .screen { display: flex; flex-direction: column; height: 100%; min-width: 0; }
  .top { display: flex; align-items: center; gap: var(--space-3); min-height: 59px; padding: 14px 24px; border-bottom: 1px solid var(--line); }
  h1 { color: var(--ink); white-space: nowrap; }
  .spacer { flex: 1; }
  .content { flex: 1; min-height: 0; overflow-y: auto; padding: 24px; display: flex; flex-direction: column; gap: var(--space-6); }
  .section-head { display: flex; align-items: baseline; gap: var(--space-4); margin-bottom: 14px; flex-wrap: wrap; }
  .section-head .spacer { flex: 1; }
  .section-head h2 { color: var(--ink); }
  .hint { font-size: var(--text-sm); color: var(--muted); max-width: 640px; }

  .pending { display: flex; align-items: center; gap: var(--space-3); padding: 12px 16px; border: 1px solid var(--app-amber); background: var(--bg-raised); }
  .pending .text { flex: 1; display: flex; flex-direction: column; gap: var(--space-1); }
  .question { font-size: var(--text-md); color: var(--ink-strong); }
  .detail { font-size: var(--text-sm); color: var(--muted); }
  .count { font-family: var(--font-mono); color: var(--app-amber-ink); }

  .cards { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: var(--space-3); }
  .card { display: flex; flex-direction: column; gap: var(--space-3); padding: 14px 16px; border: 1px solid var(--line); background: var(--bg-raised); transition: border-color var(--dur) var(--ease); }
  .card:hover { border-color: var(--line-strong); }
  .card.active { border-color: var(--accent); }
  .card.dragging { opacity: 0.5; }
  .card-head { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .icon { display: inline-flex; color: var(--muted); }
  .active .icon { color: var(--accent-ink); }
  .name { margin: 0; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-display); font-size: var(--text-lg); font-weight: 600; color: var(--ink-strong); }
  .name:focus-visible { outline: 1px solid var(--ink); outline-offset: 2px; }
  .badge { font-family: var(--font-mono); font-size: var(--text-2xs); letter-spacing: 0.12em; text-transform: uppercase; color: var(--accent-ink); border: 1px solid var(--accent); padding: 2px 6px; }
  .warn { margin: 0; font-size: var(--text-sm); color: var(--app-amber-ink); }
  .grip { cursor: grab; color: var(--faint); }
  .card-foot { display: flex; align-items: center; gap: var(--space-1); }
  kbd { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--muted); border: 1px solid var(--line); padding: 2px 6px; }
  .faint { font-size: var(--text-xs); color: var(--faint); }
  .current-sketch { border: 1px solid var(--line); background: var(--bg-raised); padding: 16px; }
  .body { margin: 0; font-size: var(--text-md); line-height: 1.5; color: var(--muted); }
  @media (prefers-reduced-motion: reduce) { .card { transition: none; } }
</style>
