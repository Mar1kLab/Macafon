<script lang="ts">
  // 03 / SOUNDPAD (PRD §7.6): a library of sound files played into Macafon Microphone and
  // the headphones at once. Categories are the folder's subfolders; files are added with a
  // button, by dropping them on the window or by copying them into the folder (SND-001).
  // Each sound has its own volume and hotkeys (SND-004/005); the bottom bar is the transport.
  import { tick } from 'svelte';
  import {
    FolderOpen,
    Headphones,
    Keyboard,
    Mic,
    Pencil,
    Play,
    Plus,
    Search,
    Shuffle,
    Square,
    Star,
    Trash2,
    TriangleAlert,
    Upload,
  } from '@lucide/svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import Dropdown from '../lib/components/Dropdown.svelte';
  import RulerSlider from '../lib/components/RulerSlider.svelte';
  import Ticks from '../lib/components/Ticks.svelte';
  import Toggle from '../lib/components/Toggle.svelte';
  import LibraryFolder from '../lib/soundpad/LibraryFolder.svelte';
  import SoundDialog from '../lib/soundpad/SoundDialog.svelte';
  import Transport from '../lib/soundpad/Transport.svelte';
  import {
    FAVORITES,
    UNCATEGORIZED,
    formatDuration,
    inScope,
    matches,
    saveSort,
    savedSort,
    sortSounds,
    type SoundScope,
    type SoundSort,
  } from '../lib/soundpad/format';
  import { indexShortcuts } from '../lib/settings/shortcut';
  import { daemon } from '../lib/state';
  import { t } from '../lib/i18n';
  import type { SoundInfo } from '../lib/ipc/protocol';

  const SOUND_MIME = 'application/x-macafon-sound';
  const ACCEPT = '.mp3,.wav,.ogg,.flac,.m4a,.aac,audio/*';

  const soundpad = $derived(daemon.soundpad);
  const prefs = $derived(soundpad?.prefs);
  const playback = $derived(daemon.soundpadPlayback);

  let scope = $state<SoundScope>(null);
  let query = $state('');
  let sort = $state<SoundSort>(savedSort());
  let selected = $state<string | null>(null);
  let editing = $state<string | null>(null);
  const SCREEN_TARGET = Symbol('screen');
  let dropTarget = $state<SoundScope | typeof SCREEN_TARGET>(null);
  let fileInput: HTMLInputElement | undefined = $state();
  let listElement: HTMLUListElement | undefined = $state();
  let categoryDialog = $state<{ from: string | null; name: string } | null>(null);
  let deletingCategory = $state<string | null>(null);
  let deletingSound = $state<SoundInfo | null>(null);

  const sounds = $derived(soundpad?.sounds ?? []);
  const visible = $derived(sortSounds(sounds.filter((sound) => inScope(sound, scope) && matches(sound, query)), sort));
  // Rows are drawn a page at a time, so a library of thousands of sounds opens at once; a new
  // filter or order starts from the first page again.
  const ROWS_PAGE = 100;
  let shown = $state(ROWS_PAGE);
  const rendered = $derived(visible.slice(0, shown));
  $effect(() => {
    void query;
    void scope;
    void sort;
    shown = ROWS_PAGE;
  });

  function onrowsscroll(event: Event) {
    const list = event.currentTarget as HTMLElement;
    if (shown < visible.length && list.scrollTop + list.clientHeight > list.scrollHeight - list.clientHeight) shown += ROWS_PAGE;
  }
  const editingSound = $derived(editing ? sounds.find((sound) => sound.id === editing) : undefined);
  /** Uploads land in the category on screen; favorites and "all" add to the folder itself. */
  const uploadCategory = $derived(scope && scope !== FAVORITES ? scope : null);

  const scopes = $derived([
    { value: null as SoundScope, label: t('soundpad.all'), count: sounds.length },
    { value: FAVORITES as SoundScope, label: t('soundpad.favorites'), count: sounds.filter((s) => s.favorite).length },
    { value: UNCATEGORIZED as SoundScope, label: t('soundpad.uncategorized'), count: sounds.filter((s) => s.category === null).length },
  ]);
  const categories = $derived(
    (soundpad?.categories ?? []).map((name) => ({ name, count: sounds.filter((s) => s.category === name).length })),
  );
  const sortOptions = $derived([
    { value: 'newest' as SoundSort, label: t('soundpad.sortNewest') },
    { value: 'oldest' as SoundSort, label: t('soundpad.sortOldest') },
    { value: 'name' as SoundSort, label: t('soundpad.sortName') },
    { value: 'plays' as SoundSort, label: t('soundpad.sortPlays') },
    { value: 'duration' as SoundSort, label: t('soundpad.sortDuration') },
    { value: 'category' as SoundSort, label: t('soundpad.sortCategory') },
  ]);
  const monitorOptions = $derived([
    { value: null as string | null, label: t('soundpad.followDefault') },
    ...(daemon.mixer?.outputs ?? []).map((output) => ({ value: output.name as string | null, label: output.description })),
  ]);

  const shortcuts = $derived(indexShortcuts(daemon.hotkeys?.bindings ?? []));

  function toggle(sound: SoundInfo) {
    if (playback?.sound_id === sound.id) daemon.stopSound();
    else daemon.playSound(sound.id);
  }

  async function upload(files: FileList | File[] | null | undefined, category: string | null) {
    const list = [...(files ?? [])];
    if (list.length === 0) return;
    const added = await daemon.uploadSounds(list, category);
    if (added > 0) daemon.toast(t('soundpad.added', { count: added }));
  }

  function setFavorite(sound: SoundInfo) {
    void daemon.setSoundpad(daemon.client.call('soundpad.set_sound', { id: sound.id, patch: { favorite: !sound.favorite } }));
  }

  async function saveCategory() {
    const dialog = categoryDialog;
    const name = dialog?.name.trim();
    if (!dialog || !name) return;
    categoryDialog = null;
    const request =
      dialog.from === null
        ? daemon.client.call('soundpad.create_category', { name })
        : daemon.client.call('soundpad.rename_category', { from: dialog.from, to: name });
    if ((await daemon.setSoundpad(request)) && dialog.from !== null && scope === dialog.from) scope = name;
  }

  async function deleteCategory() {
    const name = deletingCategory;
    deletingCategory = null;
    if (name === null) return;
    if ((await daemon.setSoundpad(daemon.client.call('soundpad.delete_category', { name }))) && scope === name) scope = null;
  }

  async function deleteSound() {
    const sound = deletingSound;
    deletingSound = null;
    if (sound) await daemon.setSoundpad(daemon.client.call('soundpad.delete', { id: sound.id }));
  }

  // --- Drag & drop: files from the file manager, sounds between categories ----------------

  const hasFiles = (event: DragEvent) => event.dataTransfer?.types.includes('Files') ?? false;
  const hasSound = (event: DragEvent) => event.dataTransfer?.types.includes(SOUND_MIME) ?? false;

  function ondragover(event: DragEvent, target: SoundScope | typeof SCREEN_TARGET) {
    if (!hasFiles(event) && !(hasSound(event) && target !== SCREEN_TARGET && target !== FAVORITES && target !== null)) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.dataTransfer) event.dataTransfer.dropEffect = hasFiles(event) ? 'copy' : 'move';
    dropTarget = target;
  }

  function ondrop(event: DragEvent, target: SoundScope | typeof SCREEN_TARGET) {
    event.preventDefault();
    event.stopPropagation();
    dropTarget = null;
    const category = target === SCREEN_TARGET ? uploadCategory : target === FAVORITES || target === UNCATEGORIZED ? null : target;
    const moving = event.dataTransfer?.getData(SOUND_MIME);
    if (moving) {
      if (target === SCREEN_TARGET || target === FAVORITES || target === null) return;
      const sound = sounds.find((item) => item.id === moving);
      if (sound && sound.category !== category) void daemon.setSoundpad(daemon.client.call('soundpad.move', { id: moving, category }));
      return;
    }
    void upload(event.dataTransfer?.files, category);
  }

  // --- Keyboard: arrows move, Enter plays, F2 edits, Delete deletes (PRD §9.8) -----------

  async function focusRow(index: number) {
    const target = Math.min(Math.max(index, 0), visible.length - 1);
    const sound = visible[target];
    if (!sound) return;
    if (target >= shown) shown = target + ROWS_PAGE;
    selected = sound.id;
    await tick();
    listElement?.querySelector<HTMLElement>(`[data-id="${CSS.escape(sound.id)}"]`)?.focus();
  }

  function onrowkey(event: KeyboardEvent, sound: SoundInfo, index: number) {
    if (event.target !== event.currentTarget) return;
    const actions: Record<string, () => void> = {
      Enter: () => toggle(sound),
      ' ': () => toggle(sound),
      ArrowDown: () => void focusRow(index + 1),
      ArrowUp: () => void focusRow(index - 1),
      Home: () => void focusRow(0),
      End: () => void focusRow(visible.length - 1),
      F2: () => (editing = sound.id),
      Delete: () => (deletingSound = sound),
    };
    const action = actions[event.key];
    if (!action) return;
    event.preventDefault();
    action();
  }
</script>

{#if soundpad && prefs}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="screen"
    ondragover={(event) => ondragover(event, SCREEN_TARGET)}
    ondragleave={(event) => {
      if (event.currentTarget === event.target || !event.relatedTarget) dropTarget = null;
    }}
    ondrop={(event) => ondrop(event, SCREEN_TARGET)}
  >
    <header class="top">
      <h1 class="hud-label">{t('soundpad.section')}</h1>
      <label class="search">
        <Search size={14} strokeWidth={1.5} />
        <span class="visually-hidden">{t('soundpad.search')}</span>
        <input type="search" bind:value={query} placeholder={t('soundpad.searchPlaceholder')} aria-label={t('soundpad.search')} />
      </label>
      <span class="hud-label">{t('soundpad.sort')}</span>
      <Dropdown label={t('soundpad.sort')} options={sortOptions} value={sort} width="170px" onchange={(value) => {
        sort = value;
        saveSort(value);
      }} />
      <span class="spacer"></span>
      <button class="btn tool" title={t('soundpad.randomHint')} aria-label={t('soundpad.random')} disabled={visible.every((sound) => sound.error)} onclick={() => {
        const pool = visible.filter((sound) => !sound.error);
        const pick = pool[Math.floor(Math.random() * pool.length)];
        if (pick) daemon.playSound(pick.id);
      }}><Shuffle size={13} strokeWidth={1.5} /><span class="label">{t('soundpad.random')}</span></button>
      <button class="btn tool" title={soundpad.folder} aria-label={t('soundpad.openFolder')} onclick={() => daemon.run(daemon.client.call('soundpad.open_folder'))}><FolderOpen size={13} strokeWidth={1.5} /><span class="label">{t('soundpad.openFolder')}</span></button>
      <LibraryFolder variant="button" />
      <button class="btn invert" onclick={() => fileInput?.click()}><Plus size={13} strokeWidth={1.5} />{t('soundpad.addFiles')}</button>
      <input
        bind:this={fileInput}
        class="visually-hidden"
        type="file"
        multiple
        accept={ACCEPT}
        tabindex="-1"
        aria-hidden="true"
        onchange={(event) => {
          const input = event.currentTarget;
          void upload(input.files, uploadCategory).then(() => (input.value = ''));
        }}
      />
    </header>

    {#if !soundpad.microphone_ready}
      <p class="banner" role="status"><TriangleAlert size={14} strokeWidth={1.5} />{t('soundpad.cableMissing')}</p>
    {/if}

    <div class="body">
      <aside class="side">
        <section class="categories" aria-label={t('soundpad.categories')}>
          <ul>
            {#each scopes as item (item.value)}
              <li>
                <button
                  class="scope"
                  class:active={scope === item.value}
                  class:drop={dropTarget === item.value && item.value !== null}
                  aria-current={scope === item.value ? 'true' : undefined}
                  onclick={() => (scope = item.value)}
                  ondragover={(event) => ondragover(event, item.value)}
                  ondrop={(event) => ondrop(event, item.value)}
                >
                  {#if item.value === FAVORITES}<Star size={13} strokeWidth={1.5} />{/if}
                  <span class="label">{item.label}</span><span class="count value">{item.count}</span>
                </button>
              </li>
            {/each}
          </ul>
          <h2 class="hud-label">{t('soundpad.categories')}</h2>
          <ul>
            {#each categories as category (category.name)}
              <li class="category">
                <button
                  class="scope"
                  class:active={scope === category.name}
                  class:drop={dropTarget === category.name}
                  aria-current={scope === category.name ? 'true' : undefined}
                  onclick={() => (scope = category.name)}
                  ondragover={(event) => ondragover(event, category.name)}
                  ondrop={(event) => ondrop(event, category.name)}
                >
                  <span class="label">{category.name}</span><span class="count value">{category.count}</span>
                </button>
                <span class="tools">
                  <button class="btn icon ghost small" aria-label={`${t('soundpad.renameCategory')}: ${category.name}`} title={t('soundpad.renameCategory')} onclick={() => (categoryDialog = { from: category.name, name: category.name })}><Pencil size={12} strokeWidth={1.5} /></button>
                  <button class="btn icon ghost small" aria-label={`${t('soundpad.deleteCategory')}: ${category.name}`} title={t('soundpad.deleteCategory')} onclick={() => (deletingCategory = category.name)}><Trash2 size={12} strokeWidth={1.5} /></button>
                </span>
              </li>
            {/each}
          </ul>
          <button class="new" onclick={() => (categoryDialog = { from: null, name: '' })}><Plus size={13} strokeWidth={1.5} /><span class="hud-label">{t('soundpad.newCategory')}</span></button>
        </section>

        <section class="output" aria-label={t('soundpad.output')}>
          <h2 class="hud-label">{t('soundpad.output')}</h2>
          <div class="target">
            <Mic size={14} strokeWidth={1.5} />
            <span class="name" title={t('soundpad.toMicrophoneHint')}>{t('soundpad.toMicrophone')}</span>
            <Toggle checked={prefs.to_microphone} label={t('soundpad.toMicrophone')} color="violet" onchange={(to_microphone) => daemon.setSoundpadPrefs({ to_microphone })} />
          </div>
          <RulerSlider compact label={t('soundpad.microphoneVolume')} value={prefs.microphone_volume_pct} min={0} max={200} step={1} defaultValue={100} origin={100} color="violet" disabled={!prefs.to_microphone} format={(value) => `${value}%`} onchange={(microphone_volume_pct) => daemon.setSoundpadPrefs({ microphone_volume_pct })} />
          <div class="target">
            <Headphones size={14} strokeWidth={1.5} />
            <span class="name" title={t('soundpad.toHeadphonesHint')}>{t('soundpad.toHeadphones')}</span>
            <Toggle checked={prefs.to_headphones} label={t('soundpad.toHeadphones')} color="teal" onchange={(to_headphones) => daemon.setSoundpadPrefs({ to_headphones })} />
          </div>
          <RulerSlider compact label={t('soundpad.headphonesVolume')} value={prefs.headphones_volume_pct} min={0} max={200} step={1} defaultValue={70} origin={100} color="teal" disabled={!prefs.to_headphones} format={(value) => `${value}%`} onchange={(headphones_volume_pct) => daemon.setSoundpadPrefs({ headphones_volume_pct })} />
          <Dropdown label={t('soundpad.monitorDevice')} options={monitorOptions} value={prefs.monitor_device} disabled={!prefs.to_headphones} onchange={(device) => daemon.setSoundpad(daemon.client.call('soundpad.set_monitor_device', { device }))} />
          <label class="target" title={t('soundpad.muteVoiceHint')}>
            <span class="name">{t('soundpad.muteVoice')}</span>
            <Toggle checked={prefs.mute_voice_while_playing} label={t('soundpad.muteVoice')} color="orange" onchange={(mute_voice_while_playing) => daemon.setSoundpadPrefs({ mute_voice_while_playing })} />
          </label>
        </section>
      </aside>

      <section class="list" aria-label={t('soundpad.section')}>
        {#if sounds.length === 0}
          <div class="empty">
            <div class="intro">
              <Ticks />
              <span class="hud-label">{t('soundpad.emptyLabel')}</span>
              <h2>{t('soundpad.emptyTitle')} <em>{t('soundpad.emptyAccent')}</em>.</h2>
              <p>{t('soundpad.emptyBody')}</p>
              <p class="note">{t('soundpad.emptyNote', { folder: soundpad.folder })}</p>
              <div class="actions">
                <button class="btn invert" onclick={() => fileInput?.click()}><Plus size={13} strokeWidth={1.5} />{t('soundpad.addFiles')}</button>
                <button class="btn" onclick={() => daemon.run(daemon.client.call('soundpad.open_folder'))}><FolderOpen size={13} strokeWidth={1.5} />{t('soundpad.openFolder')}</button>
                <LibraryFolder variant="button" />
              </div>
            </div>
          </div>
        {:else}
          <div class="head hud-label" aria-hidden="true">
            <span></span>
            <span>{t('soundpad.columns.name')}</span>
            <span>{t('soundpad.columns.category')}</span>
            <span class="right">{t('soundpad.columns.length')}</span>
            <span>{t('soundpad.columns.hotkey')}</span>
            <span></span>
          </div>
          <ul class="rows" bind:this={listElement} role="listbox" aria-label={t('soundpad.section')} onscroll={onrowsscroll}>
            {#each rendered as sound, index (sound.id)}
              {@const playing = playback?.sound_id === sound.id}
            {@const shortcut = shortcuts.get(`soundpad_play.${sound.id}`) ?? null}
              <li
                class="row"
                class:selected={selected === sound.id}
                class:playing
                class:broken={sound.error !== null}
                data-id={sound.id}
                role="option"
                aria-selected={selected === sound.id}
                tabindex="0"
                draggable="true"
                aria-label={sound.name}
                title={sound.error ? t('soundpad.broken', { error: sound.error }) : undefined}
                style:--progress={playing && playback ? `${Math.min(playback.position_ms / Math.max(playback.duration_ms, 1), 1) * 100}%` : '0%'}
                onclick={() => (selected = sound.id)}
                ondblclick={() => toggle(sound)}
                onkeydown={(event) => onrowkey(event, sound, index)}
                ondragstart={(event) => {
                  event.dataTransfer?.setData(SOUND_MIME, sound.id);
                  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
                }}
                ondragend={() => (dropTarget = null)}
              >
                <button
                  class="play"
                  class:on={playing}
                  disabled={sound.error !== null}
                  tabindex="-1"
                  aria-label={playing ? t('soundpad.stop') : t('soundpad.play', { name: sound.name })}
                  onclick={(event) => {
                    event.stopPropagation();
                    toggle(sound);
                  }}
                >
                  {#if playing}<Square size={12} strokeWidth={1.5} />{:else if sound.error}<TriangleAlert size={13} strokeWidth={1.5} />{:else}<Play size={13} strokeWidth={1.5} />{/if}
                </button>
                <span class="title">
                  <b>{sound.name}</b>
                  {#if sound.volume_pct !== 100}<span class="badge">{Math.round(sound.volume_pct)}%</span>{/if}
                </span>
                <span class="cat">{sound.category ?? ''}</span>
                <span class="len value">{formatDuration(sound.duration_ms, true)}</span>
                <span class="key">
                  {#if shortcut}<kbd>{shortcut}</kbd>{:else}<button class="assign" tabindex="-1" aria-label={`${t('soundpad.hotkey')}: ${sound.name}`} onclick={(event) => {
                    event.stopPropagation();
                    editing = sound.id;
                  }}><Keyboard size={12} strokeWidth={1.5} /></button>{/if}
                </span>
                <span class="tools">
                  <button class="btn icon ghost small fav" class:on={sound.favorite} tabindex="-1" aria-pressed={sound.favorite} aria-label={sound.favorite ? t('soundpad.unfavorite') : t('soundpad.favorite')} title={sound.favorite ? t('soundpad.unfavorite') : t('soundpad.favorite')} onclick={(event) => {
                    event.stopPropagation();
                    setFavorite(sound);
                  }}><Star size={13} strokeWidth={1.5} /></button>
                  <button class="btn icon ghost small" tabindex="-1" aria-label={t('soundpad.edit', { name: sound.name })} title={t('soundpad.edit', { name: sound.name })} onclick={(event) => {
                    event.stopPropagation();
                    editing = sound.id;
                  }}><Pencil size={13} strokeWidth={1.5} /></button>
                </span>
              </li>
            {:else}
              <li class="none" role="presentation">{query ? t('soundpad.noMatches', { query }) : t('soundpad.noneInCategory')}</li>
            {/each}
          </ul>
        {/if}
        {#each Object.entries(daemon.uploads) as [name, progress] (name)}
          <p class="uploading" role="status">
            <Upload size={13} strokeWidth={1.5} /><span>{t('soundpad.uploading', { name })}</span>
            <span class="bar"><span style:width="{progress * 100}%"></span></span>
          </p>
        {/each}
      </section>
    </div>

    <Transport />

    {#if dropTarget !== null}
      <div class="dropzone" aria-hidden="true">
        <Ticks color="var(--accent)" />
        <Upload size={20} strokeWidth={1.5} />
        <span class="hud-label">
          {dropTarget === SCREEN_TARGET
            ? uploadCategory
              ? t('soundpad.dropTo', { name: uploadCategory })
              : t('soundpad.drop')
            : dropTarget === FAVORITES || dropTarget === UNCATEGORIZED
              ? t('soundpad.drop')
              : t('soundpad.dropTo', { name: dropTarget })}
        </span>
      </div>
    {/if}
  </div>
{:else}
  <div class="loading" role="status"><span class="hud-label">{t('daemon.loading')}</span></div>
{/if}

{#if editingSound}
  {#key editingSound.id}
    <SoundDialog sound={editingSound} onclose={() => (editing = null)} onrenamed={(id) => (editing = selected = id)} />
  {/key}
{/if}

<Dialog open={categoryDialog !== null} title={categoryDialog?.from ? t('soundpad.renameCategory') : t('soundpad.newCategory')} onclose={() => (categoryDialog = null)}>
  {#if categoryDialog}
    <form
      class="field"
      onsubmit={(event) => {
        event.preventDefault();
        void saveCategory();
      }}
    >
      <span class="hud-label">{t('soundpad.categoryName')}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input" bind:value={categoryDialog.name} maxlength={64} autofocus />
    </form>
  {/if}
  {#snippet actions()}
    <button class="btn" onclick={() => (categoryDialog = null)}>{t('common.cancel')}</button>
    <button class="btn invert" disabled={!categoryDialog?.name.trim()} onclick={saveCategory}>{t('soundpad.save')}</button>
  {/snippet}
</Dialog>

<Dialog open={deletingCategory !== null} title={t('soundpad.deleteCategory')} onclose={() => (deletingCategory = null)}>
  <p>{t('soundpad.deleteCategoryBody', { name: deletingCategory ?? '' })}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (deletingCategory = null)}>{t('common.cancel')}</button>
    <button class="btn danger on" onclick={deleteCategory}>{t('soundpad.delete')}</button>
  {/snippet}
</Dialog>

<Dialog open={deletingSound !== null} title={t('soundpad.deleteTitle')} onclose={() => (deletingSound = null)}>
  <p>{t('soundpad.deleteBody', { name: deletingSound?.name ?? '' })}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (deletingSound = null)}>{t('common.cancel')}</button>
    <button class="btn danger on" onclick={deleteSound}>{t('soundpad.delete')}</button>
  {/snippet}
</Dialog>

<style>
  .screen {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    container: soundpad / inline-size;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 10px 12px;
    min-height: 59px;
    padding: 14px 24px;
    border-bottom: 1px solid var(--line);
  }

  h1 {
    color: var(--ink);
    white-space: nowrap;
    margin-right: 8px;
  }

  /* A narrow window keeps the toolbar on one line: its tools show only their icons. A container
     query, as media queries do not see the UI zoom. */
  @container soundpad (max-width: 1176px) {
    .top :global(.tool .label) {
      display: none;
    }
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 240px;
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--muted);
    transition: border-color var(--dur) var(--ease);
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
    user-select: text;
  }

  .search input::placeholder {
    color: var(--faint);
  }

  .spacer {
    flex: 1;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 24px;
    border-bottom: 1px solid var(--line);
    font-size: 13px;
    color: var(--app-amber-ink);
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .side {
    width: 248px;
    flex: none;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--line);
    background: var(--bg-raised);
    min-height: 0;
  }

  .categories {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    align-content: start;
    gap: 4px;
  }

  .categories h2 {
    margin: 12px 0 4px 8px;
  }

  .categories ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 2px;
  }

  .category {
    position: relative;
  }

  .scope {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border: 1px solid transparent;
    color: var(--muted);
    text-align: left;
    transition:
      color var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  .scope:hover {
    color: var(--ink);
    border-color: var(--line);
  }

  .scope.active {
    color: var(--ink-strong);
    border-color: var(--line-strong);
    background: var(--bg-sunken);
  }

  .scope.drop {
    border-color: var(--accent);
    color: var(--accent-ink);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .count {
    color: var(--faint);
    font-size: 11px;
  }

  .category .tools {
    position: absolute;
    right: 32px;
    top: 4px;
    display: none;
    background: var(--bg-sunken);
  }

  .category:hover .tools,
  .category:focus-within .tools {
    display: flex;
  }

  .new {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 32px;
    margin-top: 6px;
    border: 1px dashed var(--line-strong);
    color: var(--muted);
    transition: border-color var(--dur) var(--ease);
  }

  .new:hover {
    border-color: var(--ink);
    color: var(--ink);
  }

  .output {
    display: grid;
    gap: 8px;
    padding: 14px 16px 16px;
    border-top: 1px solid var(--line);
  }

  .target {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    margin-top: 4px;
  }

  .target .name {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--ink);
  }

  .list {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .head,
  .row {
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr) minmax(0, 160px) 72px 120px 64px;
    align-items: center;
    column-gap: 12px;
  }

  .head {
    padding: 10px 29px 8px;
    border-bottom: 1px solid var(--line);
    font-size: 10px;
  }

  .right {
    text-align: right;
  }

  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 10px 16px 16px;
    display: grid;
    align-content: start;
    gap: 6px;
  }

  .row {
    position: relative;
    height: 46px;
    padding: 0 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    cursor: default;
    transition:
      border-color var(--dur) var(--ease),
      background var(--dur) var(--ease);
  }

  .row:hover {
    border-color: var(--line-strong);
    background: var(--bg-raised);
  }

  .row.selected {
    border-color: var(--ink);
    background: var(--bg-raised);
  }

  .row.playing {
    border-color: var(--accent);
    background: var(--bg-raised);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .row.playing::after {
    content: '';
    position: absolute;
    left: 0;
    bottom: -1px;
    width: var(--progress);
    height: 2px;
    background: var(--accent);
  }

  .row.broken .title b {
    color: var(--faint);
  }

  .play {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--line);
    color: var(--muted);
    background: var(--bg-sunken);
    transition:
      color var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  .play:hover:not(:disabled),
  .row:hover .play:not(:disabled) {
    color: var(--ink-strong);
    border-color: var(--line-strong);
  }

  .play.on {
    color: var(--accent-ink);
    border-color: var(--accent);
  }

  .broken .play {
    color: var(--danger);
  }

  .title {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .title b {
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--ink);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .playing .title b {
    color: var(--ink-strong);
  }

  .cat {
    font-size: 12px;
    color: var(--muted);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .len {
    text-align: right;
    color: var(--muted);
  }

  .key {
    min-width: 0;
    display: flex;
  }

  kbd {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 1px 6px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink);
  }

  .assign {
    display: none;
    color: var(--faint);
  }

  .row:hover .assign,
  .row.selected .assign {
    display: inline-flex;
  }

  .assign:hover {
    color: var(--ink);
  }

  .tools {
    display: flex;
    justify-content: flex-end;
  }

  .fav {
    color: var(--faint);
  }

  .fav.on {
    color: var(--app-amber-ink);
    border-color: transparent;
  }

  .fav.on :global(svg) {
    fill: var(--app-amber);
  }

  .none {
    padding: 24px 12px;
    color: var(--muted);
    font-size: 13px;
  }

  .uploading {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 24px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--muted);
  }

  .bar {
    flex: 1;
    max-width: 240px;
    height: 4px;
    background: var(--bg-sunken);
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .empty {
    flex: 1;
    display: flex;
    align-items: safe center;
    justify-content: center;
    overflow-y: auto;
    padding: 24px;
  }

  .intro {
    position: relative;
    display: grid;
    gap: 16px;
    justify-items: start;
    width: 560px;
    padding: 32px;
    border: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .intro h2 {
    font-family: var(--font-display);
    font-size: 36px;
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1.1;
    color: var(--ink-strong);
  }

  .intro em {
    font-family: var(--font-serif);
    font-weight: 400;
    letter-spacing: 0;
  }

  .intro .note {
    font-size: 13px;
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  .dropzone {
    position: absolute;
    inset: 72px 24px 80px 272px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    border: 1px dashed var(--accent);
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    color: var(--accent-ink);
    pointer-events: none;
  }

  .dropzone .hud-label {
    color: var(--accent-ink);
  }

  .loading {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .field {
    display: grid;
    gap: 8px;
  }

  .input {
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--ink);
    font: inherit;
    user-select: text;
  }
</style>
