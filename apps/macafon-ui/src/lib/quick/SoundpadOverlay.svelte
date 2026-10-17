<script lang="ts" module>
  /** Where the user was in the sounds: kept across openings and UI restarts. */
  interface Place {
    scope: string | null;
    query: string;
    scrollTop: number;
  }

  const PLACE_KEY = 'macafon.overlay.soundpad';

  function loadPlace(): Place {
    try {
      const saved = JSON.parse(localStorage.getItem(PLACE_KEY) ?? 'null') as Partial<Place> | null;
      return {
        scope: typeof saved?.scope === 'string' ? saved.scope : null,
        query: typeof saved?.query === 'string' ? saved.query : '',
        scrollTop: typeof saved?.scrollTop === 'number' ? saved.scrollTop : 0,
      };
    } catch {
      return { scope: null, query: '', scrollTop: 0 };
    }
  }
</script>

<script lang="ts">
  // The soundpad in the hotkey overlay (§7.6, SND-006): collapsed on every opening, expanded by
  // its own button into searchable pads with a category filter. The category, search and scroll
  // position are remembered, so a long library opens where the user left it.
  import { tick } from 'svelte';
  import { AudioWaveform, ChevronDown, ChevronUp, Search, Square, Star } from '@lucide/svelte';
  import { indexShortcuts } from '../settings/shortcut';
  import { FAVORITES, inScope, matches, savedSort, sortSounds, type SoundScope } from '../soundpad/format';
  import { daemon } from '../state';
  import { t } from '../i18n';

  let { opened = 0 }: { opened?: number } = $props();

  const soundpad = $derived(daemon.soundpad);
  const playback = $derived(daemon.soundpadPlayback);
  const place = loadPlace();
  let expanded = $state(false);
  let scope = $state<SoundScope>(place.scope);
  let query = $state(place.query);
  let scrollTop = place.scrollTop;
  let list: HTMLDivElement | undefined = $state();
  let search: HTMLInputElement | undefined = $state();
  let saveFrame = 0;
  /** The order picked on the Soundpad screen, read again on every expand. */
  let order = $state(savedSort());

  // Every opening of the overlay starts with the sounds folded away.
  $effect(() => {
    void opened;
    expanded = false;
  });

  const scopes = $derived([
    { value: null as SoundScope, label: t('soundpad.all') },
    ...(soundpad?.sounds.some((sound) => sound.favorite) ? [{ value: FAVORITES as SoundScope, label: t('soundpad.favorites') }] : []),
    ...(soundpad?.categories ?? []).map((category) => ({ value: category as SoundScope, label: category })),
  ]);
  // A category that is gone falls back to every sound.
  const activeScope = $derived(scopes.some((item) => item.value === scope) ? scope : null);
  const pads = $derived(
    sortSounds(
      (soundpad?.sounds ?? []).filter((sound) => !sound.error && inScope(sound, activeScope) && matches(sound, query)),
      order,
    ).sort((a, b) => Number(b.favorite) - Number(a.favorite)),
  );
  const playing = $derived(playback && soundpad?.sounds.find((sound) => sound.id === playback.sound_id));

  function save() {
    cancelAnimationFrame(saveFrame);
    saveFrame = requestAnimationFrame(() => {
      localStorage.setItem(PLACE_KEY, JSON.stringify({ scope, query, scrollTop } satisfies Place));
    });
  }

  async function toggle() {
    expanded = !expanded;
    if (!expanded) return;
    order = savedSort();
    await tick();
    if (list) list.scrollTop = scrollTop;
    search?.focus({ preventScroll: true });
  }

  function setScope(value: SoundScope) {
    scope = value;
    scrollTop = 0;
    if (list) list.scrollTop = 0;
    save();
  }

  const shortcuts = $derived(indexShortcuts(daemon.hotkeys?.bindings ?? []));
</script>

<section class="soundpad" aria-label={t('quick.soundpad')}>
  <div class="bar">
    <button class="expand" class:open={expanded} aria-expanded={expanded} aria-controls="overlay-sounds" title={expanded ? t('quick.soundpadHide') : t('quick.soundpadShowHint')} onclick={toggle}>
      <AudioWaveform size={16} strokeWidth={1.5} />
      <span>{t('quick.soundpadShow')}</span>
      <small>{soundpad?.sounds.length ?? 0}</small>
      {#if expanded}<ChevronUp size={14} strokeWidth={1.5} />{:else}<ChevronDown size={14} strokeWidth={1.5} />{/if}
    </button>
    {#if expanded}
      <label class="search">
        <Search size={13} strokeWidth={1.5} />
        <input
          bind:this={search}
          type="search"
          bind:value={query}
          placeholder={t('soundpad.searchPlaceholder')}
          aria-label={t('quick.soundpadSearch')}
          oninput={() => {
            scrollTop = 0;
            if (list) list.scrollTop = 0;
            save();
          }}
          onkeydown={(event) => {
            if (event.key === 'Enter' && pads[0]) {
              event.preventDefault();
              daemon.playSound(pads[0].id);
            }
          }}
        />
      </label>
    {/if}
    <span class="now" aria-live="polite">{playing?.name ?? ''}</span>
    <button class="round stop" disabled={!playback} aria-label={t('quick.soundpadStop')} title={t('quick.soundpadStop')} onclick={() => daemon.stopSound()}>
      <Square size={14} strokeWidth={1.5} />
    </button>
  </div>

  {#if expanded}
    <div id="overlay-sounds" class="sounds">
      <div class="scopes" role="radiogroup" aria-label={t('soundpad.categories')}>
        {#each scopes as item (item.value)}
          <button class="chip" role="radio" aria-checked={activeScope === item.value} class:active={activeScope === item.value} onclick={() => setScope(item.value)}>
            {#if item.value === FAVORITES}<Star size={11} strokeWidth={1.5} />{/if}{item.label}
          </button>
        {/each}
      </div>
      {#if (soundpad?.sounds.length ?? 0) === 0}
        <p class="empty">{t('quick.soundpadEmpty')}</p>
      {:else if pads.length === 0}
        <p class="empty">{t('quick.soundpadNoMatches', { query })}</p>
      {:else}
        <div
          class="pads"
          bind:this={list}
          onscroll={() => {
            scrollTop = list?.scrollTop ?? 0;
            save();
          }}
        >
          {#each pads as sound (sound.id)}
            {@const active = playback?.sound_id === sound.id}
            {@const shortcut = shortcuts.get(`soundpad_play.${sound.id}`) ?? null}
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

  .expand {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    flex: none;
    height: 36px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--bg-sunken);
    font-size: 12px;
    color: var(--ink);
    transition:
      border-color var(--dur) var(--ease),
      background var(--dur) var(--ease);
  }

  .expand:hover {
    border-color: var(--line-strong);
    background: var(--bg);
  }

  .expand.open {
    border-color: var(--accent);
  }

  .expand.open > :global(svg:first-child) {
    color: var(--accent-ink);
  }

  .expand small {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--muted);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    max-width: 280px;
    height: 36px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--bg-sunken);
    color: var(--muted);
  }

  .search:focus-within {
    border-color: var(--accent);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    color: var(--ink);
    outline: none;
    font-size: 12px;
    user-select: text;
  }

  .search input::placeholder {
    color: var(--faint);
  }

  .sounds {
    display: grid;
    gap: 10px;
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
    max-height: 236px;
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
    .pad,
    .expand {
      transition: none;
    }
  }
</style>
