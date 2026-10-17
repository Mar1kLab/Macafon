<script lang="ts">
  // The soundpad in the hotkey overlay (§7.6, SND-006): pads of the library, favorites first,
  // a category filter and the stop button. Shown when Settings allows it.
  import { Square, Star } from '@lucide/svelte';
  import { displayShortcut } from '../settings/shortcut';
  import { FAVORITES, inScope, sortSounds, type SoundScope } from '../soundpad/format';
  import { daemon } from '../state';
  import { t } from '../i18n';

  const soundpad = $derived(daemon.soundpad);
  const playback = $derived(daemon.soundpadPlayback);
  let scope = $state<SoundScope>(null);

  const scopes = $derived([
    { value: null as SoundScope, label: t('soundpad.all') },
    ...(soundpad?.sounds.some((sound) => sound.favorite) ? [{ value: FAVORITES as SoundScope, label: t('soundpad.favorites') }] : []),
    ...(soundpad?.categories ?? []).map((category) => ({ value: category as SoundScope, label: category })),
  ]);
  const pads = $derived(
    sortSounds((soundpad?.sounds ?? []).filter((sound) => !sound.error && inScope(sound, scope)), 'name').sort(
      (a, b) => Number(b.favorite) - Number(a.favorite),
    ),
  );
  const playing = $derived(playback && soundpad?.sounds.find((sound) => sound.id === playback.sound_id));

  function shortcutOf(id: string): string | null {
    const shortcut = daemon.hotkeys?.bindings.find((binding) => binding.action === `soundpad_play.${id}`)?.shortcut;
    return shortcut ? displayShortcut(shortcut) : null;
  }
</script>

<section class="soundpad" aria-label={t('quick.soundpad')}>
  <div class="bar">
    <span class="hud-label">{t('quick.soundpad')}</span>
    <div class="scopes" role="radiogroup" aria-label={t('soundpad.categories')}>
      {#each scopes as item (item.value)}
        <button class="chip" role="radio" aria-checked={scope === item.value} class:active={scope === item.value} onclick={() => (scope = item.value)}>
          {#if item.value === FAVORITES}<Star size={11} strokeWidth={1.5} />{/if}{item.label}
        </button>
      {/each}
    </div>
    <span class="now" aria-live="polite">{playing?.name ?? ''}</span>
    <button class="round stop" disabled={!playback} aria-label={t('quick.soundpadStop')} title={t('quick.soundpadStop')} onclick={() => daemon.stopSound()}>
      <Square size={14} strokeWidth={1.5} />
    </button>
  </div>
  {#if pads.length === 0}
    <p class="empty">{t('quick.soundpadEmpty')}</p>
  {:else}
    <div class="pads">
      {#each pads as sound (sound.id)}
        {@const active = playback?.sound_id === sound.id}
        {@const shortcut = shortcutOf(sound.id)}
        <button
          class="pad"
          class:active
          aria-label={t('quick.soundpadPlay', { name: sound.name })}
          title={sound.name}
          style:--progress={active && playback ? `${Math.min(playback.position_ms / Math.max(playback.duration_ms, 1), 1) * 100}%` : '0%'}
          onclick={() => (active ? daemon.stopSound() : daemon.playSound(sound.id))}
        >
          <span class="name">{#if sound.favorite}<Star size={11} strokeWidth={1.5} />{/if}{sound.name}</span>
          {#if shortcut}<kbd>{shortcut}</kbd>{/if}
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .soundpad {
    margin-top: 16px;
    padding-top: 14px;
    border-top: 1px solid var(--line);
    display: grid;
    gap: 10px;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }

  .scopes {
    display: flex;
    gap: 6px;
    min-width: 0;
    overflow-x: auto;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    height: 24px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 12px;
    font-size: 11px;
    color: var(--muted);
    background: var(--bg-sunken);
  }

  .chip.active {
    color: var(--ink-strong);
    border-color: var(--line-strong);
  }

  .now {
    margin-left: auto;
    max-width: 200px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 12px;
    color: var(--accent-ink);
  }

  .round {
    width: 32px;
    height: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    border: 1px solid var(--line);
    border-radius: 50%;
    color: var(--muted);
    background: var(--bg-sunken);
  }

  .round:hover:not(:disabled) {
    border-color: var(--line-strong);
    color: var(--ink-strong);
  }

  .round:disabled {
    opacity: 0.4;
  }

  .pads {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 8px;
    max-height: 176px;
    overflow-y: auto;
    padding-right: 2px;
  }

  .pad {
    position: relative;
    overflow: hidden;
    display: grid;
    align-content: space-between;
    gap: 6px;
    height: 52px;
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--bg-sunken);
    text-align: left;
    transition:
      border-color var(--dur) var(--ease),
      background var(--dur) var(--ease);
  }

  .pad:hover {
    border-color: var(--line-strong);
    background: var(--bg);
  }

  .pad.active {
    border-color: var(--accent);
  }

  .pad.active::after {
    content: '';
    position: absolute;
    left: 0;
    bottom: 0;
    width: var(--progress);
    height: 3px;
    background: var(--accent);
  }

  .name {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 12px;
    color: var(--ink);
  }

  .name :global(svg) {
    flex: none;
    color: var(--app-amber-ink);
  }

  kbd {
    justify-self: start;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--muted);
  }

  .empty {
    font-size: 13px;
    color: var(--muted);
  }

  @media (prefers-reduced-motion: reduce) {
    .pad {
      transition: none;
    }
  }
</style>
