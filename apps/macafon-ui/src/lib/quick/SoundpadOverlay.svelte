<script lang="ts" module>
  import { FAVORITES, SOUND_SORTS, savedSort, type SoundSort } from '../soundpad/format';

  /** Where the user was in the sounds: kept across openings and UI restarts. */
  interface Place {
    /** A category, or `null` for every sound. */
    scope: string | null;
    /** Only the favorites of that scope. */
    favorites: boolean;
    query: string;
    scrollTop: number;
    /** The overlay's own order; the SoundDeck screen's until the user picks one here. */
    sort: SoundSort;
  }

  const PLACE_KEY = 'macafon.overlay.soundpad';

  function loadPlace(): Place {
    try {
      const saved = JSON.parse(localStorage.getItem(PLACE_KEY) ?? 'null') as Partial<Place> | null;
      // Favorites used to be one of the scopes.
      const oldFavorites = saved?.scope === FAVORITES;
      return {
        scope: typeof saved?.scope === 'string' && !oldFavorites ? saved.scope : null,
        favorites: saved?.favorites === true || oldFavorites,
        query: typeof saved?.query === 'string' ? saved.query : '',
        scrollTop: typeof saved?.scrollTop === 'number' ? saved.scrollTop : 0,
        sort: SOUND_SORTS.includes(saved?.sort as SoundSort) ? (saved?.sort as SoundSort) : savedSort(),
      };
    } catch {
      return { scope: null, favorites: false, query: '', scrollTop: 0, sort: savedSort() };
    }
  }
</script>

<script lang="ts">
  // SoundDeck in the hotkey overlay (§7.6, SND-006), shown while its panel is open: searchable,
  // sortable pads with a Favorites chip and one list of categories (many categories do not
  // crowd the panel), a star on each pad, the playing sound's transport and the output
  // volumes. The category, favorites, search, order and scroll position are remembered, so a
  // long library opens where the user left it. Pads are drawn a page at a time, so a library of
  // thousands of sounds opens as fast as a small one.
  import { onMount, tick } from 'svelte';
  import { Headphones, Mic, Pause, Play, Search, Square, Star } from '@lucide/svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import RulerSlider from '../components/RulerSlider.svelte';
  import SeekBar from '../soundpad/SeekBar.svelte';
  import { indexShortcuts } from '../settings/shortcut';
  import { formatDuration, inScope, matches, sortOptions, sortSounds, type SoundScope } from '../soundpad/format';
  import type { SoundInfo, SoundpadOverlayView } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';

  /** `library`: the sounds, in the overlay's side panel; `player`: the transport and the output
   * volumes, docked along the bottom of the overlay. */
  let { part = 'library' }: { part?: 'library' | 'player' } = $props();

  /** Pads per page: a little more than the visible area in either view. */
  const PAGE = 60;

  const soundpad = $derived(daemon.soundpad);
  /** SND-006: pads in a grid or rows in a list. */
  const view = $derived(soundpad?.prefs.overlay_view ?? 'grid');
  const sketch = $derived(daemon.prefs?.osd.look === 'sketch');
  /** Row height and columns of the view, to load enough rows for a scroll position. */
  const metricsOf = (layout: SoundpadOverlayView) => (layout === 'list' ? { row: 32, columns: 1 } : { row: 60, columns: 3 });
  const metrics = $derived(metricsOf(view));
  const playback = $derived(daemon.soundpadPlayback);
  const playingId = $derived(playback?.sound_id ?? null);
  const place = loadPlace();
  let scope = $state<SoundScope>(place.scope);
  let favorites = $state(place.favorites);
  let query = $state(place.query);
  let scrollTop = place.scrollTop;
  let list: HTMLDivElement | undefined = $state();
  let search: HTMLInputElement | undefined = $state();
  let sort = $state<SoundSort>(place.sort);
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  const sorts = sortOptions();
  /** Enough pads to reach the remembered scroll position, then a page more on demand. */
  const first = metricsOf(daemon.soundpad?.prefs.overlay_view ?? 'grid');
  let shown = $state(Math.max(PAGE, Math.ceil(place.scrollTop / first.row + 8) * first.columns));

  const scopes = $derived([
    { value: null as SoundScope, label: t('soundpad.all') },
    ...(soundpad?.categories ?? []).map((category) => ({ value: category as SoundScope, label: category })),
  ]);
  const favoriteCount = $derived(soundpad?.sounds.filter((sound) => sound.favorite).length ?? 0);
  // A category that is gone falls back to every sound.
  const activeScope = $derived(scopes.some((item) => item.value === scope) ? scope : null);
  const pads = $derived(
    sortSounds(
      (soundpad?.sounds ?? []).filter((sound) => !sound.error && inScope(sound, activeScope) && (!favorites || sound.favorite) && matches(sound, query)),
      sort,
    ).sort((a, b) => Number(b.favorite) - Number(a.favorite)),
  );
  const visible = $derived(pads.slice(0, shown));
  const playing = $derived(playingId === null ? null : soundpad?.sounds.find((sound) => sound.id === playingId));

  onMount(() => {
    void tick().then(() => {
      if (list) list.scrollTop = scrollTop;
      if (part === 'library') search?.focus({ preventScroll: true });
    });
    return () => {
      clearTimeout(saveTimer);
      persist();
    };
  });

  function save() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(persist, 200);
  }

  function persist() {
    if (part === 'library') localStorage.setItem(PLACE_KEY, JSON.stringify({ scope, favorites, query, scrollTop, sort } satisfies Place));
  }

  function restart() {
    scrollTop = 0;
    shown = PAGE;
    if (list) list.scrollTop = 0;
    save();
  }

  function setScope(value: SoundScope) {
    scope = value;
    restart();
  }

  function setFavorites(value: boolean) {
    favorites = value;
    restart();
  }

  function setSort(value: SoundSort) {
    sort = value;
    restart();
  }

  /** The pad's star: favorites without leaving the overlay. */
  function star(sound: SoundInfo) {
    void daemon.setSoundpad(daemon.client.call('soundpad.set_sound', { id: sound.id, patch: { favorite: !sound.favorite } }));
  }

  /** Name, category, length and plays: everything the pad has no room for. */
  function describe(sound: SoundInfo): string {
    return [sound.name, sound.category, formatDuration(sound.duration_ms, true), t('soundpad.plays', { count: sound.play_count })]
      .filter(Boolean)
      .join(' · ');
  }

  function onscroll() {
    if (!list) return;
    scrollTop = list.scrollTop;
    if (list.scrollTop + list.clientHeight > list.scrollHeight - metrics.row * 2 && shown < pads.length) shown += PAGE;
    save();
  }

  const shortcuts = $derived(indexShortcuts(daemon.hotkeys?.bindings ?? []));
</script>

{#snippet player()}
  <div class="player">
    <button
      class="round"
      class:on={playback && !playback.paused}
      disabled={!playback}
      aria-label={playback?.paused ? t('soundpad.resume') : t('soundpad.pause')}
      title={playback?.paused ? t('soundpad.resume') : t('soundpad.pause')}
      onclick={() => daemon.run(daemon.client.call('soundpad.set_paused', { paused: null }))}
    >
      {#if !playback || playback.paused}<Play size={13} strokeWidth={1.5} />{:else}<Pause size={13} strokeWidth={1.5} />{/if}
    </button>
    <button class="round" disabled={!playback} aria-label={t('quick.soundpadStop')} title={t('quick.soundpadStop')} onclick={() => daemon.stopSound()}>
      <Square size={12} strokeWidth={1.5} />
    </button>
    <span class="now" aria-live="polite" title={playing?.name}>
      {#if playback}{playback.loading ? t('soundpad.loading') : (playing?.name ?? playback.sound_id)}{:else}<span class="idle">{t('soundpad.nothingPlaying')}</span>{/if}
    </span>
    <SeekBar compact />
    <span class="time">{formatDuration(playback?.position_ms ?? 0)}<span class="total">&nbsp;/ {formatDuration(playback?.duration_ms ?? null)}</span></span>
    {#if soundpad}
      <span class="volume" class:off={!soundpad.prefs.to_microphone} title={t('soundpad.microphoneVolume')}>
        <Mic size={13} strokeWidth={1.5} />
        <RulerSlider compact label={t('soundpad.microphoneVolume')} value={soundpad.prefs.microphone_volume_pct} min={0} max={200} step={1} defaultValue={100} origin={100} color="violet" disabled={!soundpad.prefs.to_microphone} format={(value) => `${value}%`} onchange={(microphone_volume_pct) => daemon.setSoundpadPrefs({ microphone_volume_pct })} />
      </span>
      <span class="volume" class:off={!soundpad.prefs.to_headphones} title={t('soundpad.headphonesVolume')}>
        <Headphones size={13} strokeWidth={1.5} />
        <RulerSlider compact label={t('soundpad.headphonesVolume')} value={soundpad.prefs.headphones_volume_pct} min={0} max={200} step={1} defaultValue={70} origin={100} color="teal" disabled={!soundpad.prefs.to_headphones} format={(value) => `${value}%`} onchange={(headphones_volume_pct) => daemon.setSoundpadPrefs({ headphones_volume_pct })} />
      </span>
    {/if}
  </div>
{/snippet}

{#if part === 'player'}
  <section class="dock" class:sketch aria-label={t('soundpad.transport')}>{@render player()}</section>
{:else}
<section id="overlay-sounds" class="soundpad" class:sketch aria-label={t('quick.soundpad')}>
  <div class="bar">
    <label class="search">
      <Search size={13} strokeWidth={1.5} />
      <input
        bind:this={search}
        type="search"
        bind:value={query}
        placeholder={t('soundpad.searchPlaceholder')}
        aria-label={t('quick.soundpadSearch')}
        oninput={restart}
        onkeydown={(event) => {
          if (event.key === 'Enter' && pads[0]) {
            event.preventDefault();
            daemon.playSound(pads[0].id);
          }
        }}
      />
    </label>
    <div class="filters">
      <button class="chip favorites" class:active={favorites} aria-pressed={favorites} title={t('quick.soundpadFavoritesHint')} onclick={() => setFavorites(!favorites)}>
        <Star size={11} strokeWidth={1.5} />{t('soundpad.favorites')}<small>{favoriteCount}</small>
      </button>
      <div class="scope"><Dropdown label={t('soundpad.categories')} options={scopes} value={activeScope} width="138px" onchange={setScope} /></div>
      <div class="sort"><Dropdown label={t('soundpad.sort')} options={sorts} value={sort} width="138px" onchange={setSort} /></div>
    </div>
  </div>

  {#if (soundpad?.sounds.length ?? 0) === 0}
    <p class="empty">{t('quick.soundpadEmpty')}</p>
  {:else if pads.length === 0}
    <p class="empty">{favorites && !query ? t('quick.soundpadNoFavorites') : t('quick.soundpadNoMatches', { query })}</p>
  {:else}
    <div class="pads" class:list={view === 'list'} bind:this={list} {onscroll}>
      {#each visible as sound (sound.id)}
        {@const active = playingId === sound.id}
        {@const shortcut = shortcuts.get(`soundpad_play.${sound.id}`) ?? null}
        <div class="pad" class:active class:favorite={sound.favorite} style:--progress={active && playback ? `${Math.min(playback.position_ms / Math.max(playback.duration_ms, 1), 1) * 100}%` : '0%'}>
          <button class="play" aria-label={t('quick.soundpadPlay', { name: sound.name })} title={describe(sound)} onclick={() => (active ? daemon.stopSound() : daemon.playSound(sound.id))}>
            <span class="name">{sound.name}</span>
            <span class="meta">
              <span class="length">{active && playback ? `${formatDuration(playback.position_ms)} / ` : ''}{formatDuration(sound.duration_ms, true)}</span>
              {#if sound.category && activeScope !== sound.category}<span class="category">{sound.category}</span>{/if}
              {#if shortcut}<kbd>{shortcut}</kbd>{/if}
            </span>
          </button>
          <button class="star" aria-pressed={sound.favorite} aria-label={sound.favorite ? t('soundpad.unfavorite') : t('soundpad.favorite')} title={sound.favorite ? t('soundpad.unfavorite') : t('soundpad.favorite')} onclick={() => star(sound)}>
            <Star size={12} strokeWidth={1.5} />
          </button>
        </div>
      {/each}
    </div>
  {/if}
</section>
{/if}

<style>
  .soundpad {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
    min-width: 0;
    flex: 1;
  }

  .dock {
    display: block;
    min-width: 0;
  }

  .bar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
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

  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .filters .sort {
    margin-left: auto;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: none;
    height: 32px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: 12px;
    font-size: 12px;
    color: var(--muted);
    background: var(--bg-sunken);
    transition: border-color var(--dur) var(--ease), color var(--dur) var(--ease);
  }

  .chip:hover {
    border-color: var(--line-strong);
    color: var(--ink);
  }

  .chip small {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--faint);
  }

  .chip :global(svg) {
    color: var(--app-amber-ink);
  }

  .chip.active {
    color: var(--ink-strong);
    border-color: var(--app-amber-ink);
  }

  .chip.active :global(svg) {
    fill: currentColor;
  }

  .player {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    min-height: 28px;
    flex-wrap: nowrap;
  }

  .player .round {
    width: 28px;
    height: 28px;
  }

  .now {
    flex: none;
    width: 120px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 12px;
    color: var(--accent-ink);
  }

  .now .idle {
    color: var(--faint);
  }

  .time {
    flex: none;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink);
  }

  .total {
    color: var(--muted);
  }

  .volume {
    flex: none;
    width: 132px;
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 6px;
    margin-left: 4px;
    color: var(--muted);
  }

  .volume.off {
    opacity: 0.5;
  }

  /* A narrower ruler: a short track and a tight readout. */
  .volume :global(.ruler.compact) {
    grid-template-columns: 1fr 34px;
    gap: 6px;
  }

  .volume :global(.ruler.compact .track) {
    height: 11px;
  }

  .volume :global(.ruler.compact .value) {
    font-size: 11px;
  }

  .sort :global(.trigger),
  .scope :global(.trigger) {
    height: 32px;
    border-radius: 12px;
    font-size: 12px;
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

  .round.on {
    color: var(--accent-ink);
    border-color: var(--accent);
  }

  .pads {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
    min-height: 0;
    flex: 1 1 0;
    grid-auto-rows: 52px;
    align-content: start;
    overflow-anchor: none;
    scrollbar-gutter: stable;
    overscroll-behavior: contain;
    overflow-y: auto;
    padding-right: 2px;
  }

  .pad {
    position: relative;
    overflow: hidden;
    height: 52px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: var(--bg-sunken);
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

  .play {
    width: 100%;
    height: 100%;
    display: grid;
    align-content: space-between;
    gap: 6px;
    padding: 8px 10px;
    border: 0;
    background: none;
    text-align: left;
  }

  .play:focus-visible,
  .star:focus-visible {
    outline: 1px solid var(--ink);
    outline-offset: -2px;
  }

  /* The star sits in the pad's corner: shown on hover and focus, always on a favorite. */
  .star {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--muted);
    opacity: 0;
    transition:
      opacity var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  .pad:hover .star,
  .star:focus-visible,
  .pad.favorite .star {
    opacity: 1;
  }

  .star:hover,
  .pad.favorite .star {
    color: var(--app-amber-ink);
  }

  .pad.favorite .star :global(svg) {
    fill: currentColor;
  }

  .name {
    min-width: 0;
    padding-right: 18px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 12px;
    color: var(--ink);
  }

  .meta {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    font-size: 10px;
    color: var(--muted);
  }

  .length {
    flex: none;
    font-family: var(--font-mono);
  }

  .pad.active .length {
    color: var(--accent-ink);
  }

  .category {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--faint);
  }

  kbd {
    flex: none;
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--muted);
  }

  .pads.list {
    grid-auto-rows: 28px;
    grid-template-columns: minmax(0, 1fr);
    gap: 4px;
  }

  /* The seek bar fills the space left by the transport and output volumes; a thin one. */
  .player > :global(.track) { flex: 1; min-width: 120px; }
  .player > :global(.track.compact) { height: 12px; }
  .player > :global(.track.compact .tick) { height: 3px; }
  .player > :global(.track.compact .tick.major) { height: 5px; }

  .list .pad {
    height: 28px;
    border-radius: 8px;
  }

  .list .play {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 34px 0 10px;
  }

  .list .star {
    top: 3px;
  }

  .list .name {
    flex: 1;
    padding-right: 0;
  }

  .list .meta {
    flex: none;
    max-width: 50%;
    flex-direction: row-reverse;
  }

  .list .meta kbd {
    margin-left: 0;
  }

  .list .length {
    width: 76px;
    text-align: right;
  }

  .list .pad.active::after {
    height: 2px;
  }

  /* Sketch: the app's own look, picked with the hotkey popup's (HK-006). */
  .sketch .search,
  .sketch .chip,
  .sketch .round,
  .sketch .pad,
  .sketch .star,
  .sketch .sort :global(.trigger),
  .sketch .scope :global(.trigger) {
    border-radius: 0;
  }

  .sketch .pad.active {
    border-color: var(--line-strong);
  }

  .sketch .pad.active::before {
    content: '';
    position: absolute;
    left: -1px;
    top: -1px;
    bottom: -1px;
    width: 2px;
    background: var(--accent);
  }

  .empty {
    font-size: 13px;
    color: var(--muted);
  }

  @media (prefers-reduced-motion: reduce) {
    .pad,
    .chip,
    .star {
      transition: none;
    }
  }
</style>
