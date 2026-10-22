<script lang="ts">
  // The overlay's display profiles (§9.9.2, DISP-002): one row per profile, a click applies it;
  // while a switch waits for Keep, Keep and Revert with the countdown replace the hint.
  import { Check } from '@lucide/svelte';
  import RotateCcw from '../icons/animated/RotateCcw.svelte';
  import { daemon } from '../state';
  import { displayShortcut } from '../settings/shortcut';
  import ProfileIcon from '../displays/ProfileIcon.svelte';
  import { useCountdown } from '../displays/countdown.svelte';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';

  const displays = $derived(daemon.displays);
  const pending = $derived(displays?.pending ?? null);
  const countdown = useCountdown(() => pending?.revert_at_unix_ms ?? null);
  const shortcutOf = (id: string) => daemon.hotkeys?.bindings.find((binding) => binding.action === `display_profile.${id}`)?.shortcut ?? null;
</script>

{#if displays}
  <section class="displays" id="overlay-displays" aria-label={t('displays.title')}>
    <div class="head">
      <span class="hud-label">{t('displays.title')}</span>
      <span class="spacer"></span>
      {#if displays.can_revert && !pending}
        <button class="round" aria-label={t('displays.revert')} {@attach tip(t('displays.revert'))} onclick={() => daemon.setDisplays(daemon.client.call('displays.revert'))}><RotateCcw size={13} strokeWidth={1.5} /></button>
      {/if}
    </div>
    {#if pending}
      <div class="pending" role="alertdialog" aria-label={t('displays.keepTitle')}>
        <span class="question">{t('displays.keepTitle')}</span>
        <span class="count" aria-live="polite">{t('displays.reverting', { seconds: countdown.seconds })}</span>
        <button class="keep" onclick={() => daemon.setDisplays(daemon.client.call('displays.confirm', { keep: true }))}>{t('displays.keep')}</button>
        <button class="undo" onclick={() => daemon.setDisplays(daemon.client.call('displays.confirm', { keep: false }))}>{t('displays.revertNow')}</button>
      </div>
    {/if}
    {#if displays.profiles.length === 0}
      <p class="empty">{t('displays.overlayEmpty')}</p>
    {:else}
      <ul>
        {#each displays.profiles as { profile, missing } (profile.id)}
          {@const active = displays.active === profile.id}
          {@const shortcut = shortcutOf(profile.id)}
          <li>
            <button class="row" class:active aria-current={active ? 'true' : undefined} {@attach tip(missing.length ? t('displays.missing', { names: missing.join(', ') }) : profile.name)} onclick={() => daemon.applyDisplayProfile(profile.id)}>
              <span class="icon"><ProfileIcon icon={profile.icon} size={15} /></span>
              <span class="name">{profile.name}</span>
              {#if missing.length}<span class="warn">{t('displays.missingShort', { count: missing.length })}</span>{/if}
              {#if shortcut}<kbd>{displayShortcut(shortcut)}</kbd>{/if}
              {#if active}<span class="mark" aria-label={t('displays.active')}><Check size={13} strokeWidth={1.75} /></span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .displays { display: flex; flex-direction: column; min-height: 0; min-width: 0; flex: 1; gap: var(--space-3); }
  .head { display: flex; align-items: center; gap: var(--space-2); flex: none; min-height: 32px; }
  .spacer { flex: 1; }
  .round { width: 32px; height: 32px; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--line); border-radius: 50%; color: var(--muted); background: var(--bg-sunken); }
  .round:hover { border-color: var(--line-strong); color: var(--ink-strong); }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-2); overflow-y: auto; min-height: 0; flex: 1; overscroll-behavior: contain; scrollbar-gutter: stable; }
  .row { width: 100%; display: flex; align-items: center; gap: var(--space-2); height: 40px; padding: 0 12px; border: 1px solid var(--line); border-radius: 12px; background: transparent; color: var(--ink); font-size: var(--text-md); text-align: left; transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease); }
  .row:hover { border-color: var(--line-strong); color: var(--ink-strong); }
  .row.active { border-color: var(--accent); }
  .row:focus-visible, .round:focus-visible, .pending button:focus-visible { outline: 1px solid var(--ink); outline-offset: 2px; }
  .icon { display: inline-flex; color: var(--muted); }
  .active .icon, .mark { color: var(--accent-ink); }
  .mark { display: inline-flex; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .warn { font-family: var(--font-mono); font-size: var(--text-2xs); color: var(--app-amber-ink); }
  kbd { font-family: var(--font-mono); font-size: var(--text-2xs); color: var(--muted); border: 1px solid var(--line); border-radius: 6px; padding: 2px 6px; }
  .pending { display: grid; grid-template-columns: 1fr auto auto; grid-template-rows: auto auto; column-gap: var(--space-2); row-gap: var(--space-half); align-items: center; padding: 10px 12px; border: 1px solid var(--app-amber); border-radius: 12px; flex: none; }
  .question { font-size: var(--text-md); color: var(--ink-strong); }
  .count { grid-row: 2; font-family: var(--font-mono); font-size: var(--text-xs); color: var(--app-amber-ink); }
  .pending button { grid-row: span 2; height: 32px; padding: 0 12px; border: 1px solid var(--line); border-radius: 10px; font-size: var(--text-sm); color: var(--ink); background: transparent; }
  .pending .keep { background: var(--accent); border-color: var(--accent); color: var(--ink-strong); }
  .empty { margin: 0; font-size: var(--text-md); color: var(--muted); }
  /* Sketch: the app's own look, picked with the hotkey popup's (HK-006). */
  :global(.sketch) .pending .keep { background: var(--bg-sunken); color: var(--accent-ink); }
  :global(.sketch) .row, :global(.sketch) .round, :global(.sketch) .pending, :global(.sketch) .pending button, :global(.sketch) kbd { border-radius: 0; }
  @media (prefers-reduced-motion: reduce) { .row { transition: none; } }
</style>
