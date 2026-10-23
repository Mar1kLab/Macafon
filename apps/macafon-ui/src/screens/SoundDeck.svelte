<script lang="ts">
  // 03 / SOUNDDECK (PRD §7.6): a library of sound files played into Macafon Microphone and
  // the headphones at once. Categories are the folder's subfolders; files are added with a
  // button, by dropping them on the window or by copying them into the folder (SND-001).
  // Files that cannot be played are moved to the Unplayable folder and listed apart, to delete.
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
  import LibraryFolder from '../lib/sounddeck/LibraryFolder.svelte';
  import SoundDialog from '../lib/sounddeck/SoundDialog.svelte';
  import Transport from '../lib/sounddeck/Transport.svelte';
  import {
    FAVORITES,
    UNCATEGORIZED,
    formatDuration,
    inScope,
    matches,
    saveSort,
    savedSort,
    sortOptions,
    sortSounds,
    type SoundScope,
    type SoundSort,
  } from '../lib/sounddeck/format';
  import { indexShortcuts } from '../lib/settings/shortcut';
  import { daemon } from '../lib/state';
  import { t } from '../lib/i18n';
  import type { SoundInfo } from '../lib/ipc/protocol';
  import SettingsLink from '../lib/components/SettingsLink.svelte';
  import { tip } from '../lib/components/tooltip.svelte';
  import Field from '../lib/components/Field.svelte';
  import Input from '../lib/components/Input.svelte';
  import PageHeader from '../lib/components/PageHeader.svelte';
  import Loader from '../lib/components/Loader.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';

  const SOUND_MIME = 'application/x-macafon-sound';
  const ACCEPT = '.mp3,.wav,.ogg,.flac,.m4a,.aac,audio/*';

  const sounddeck = $derived(daemon.sounddeck);
  const prefs = $derived(sounddeck?.prefs);
  const playback = $derived(daemon.sounddeckPlayback);

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

  const sounds = $derived(sounddeck?.sounds ?? []);
  /** Not a folder name: the daemon refuses `Unplayable` as a category. */
  const PROBLEMS = '\u0000unplayable';
  const problems = $derived(sounddeck?.problem_sounds ?? []);
  let confirmProblems = $state(false);
  // The list emptied: back to every sound.
  $effect(() => {
    if (scope === PROBLEMS && problems.length === 0) scope = null;
  });

  async function deleteProblems() {
    confirmProblems = false;
    await daemon.setSoundDeck(daemon.client.call('sounddeck.delete_problem_sounds'));
  }
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
  // Favorites and Unplayable are views, not folders: files added there go to Uncategorized.
  const uploadCategory = $derived(scope && scope !== FAVORITES && scope !== PROBLEMS ? scope : null);

  const scopes = $derived([
    { value: null as SoundScope, label: t('sounddeck.all'), count: sounds.length },
    { value: FAVORITES as SoundScope, label: t('sounddeck.favorites'), count: sounds.filter((s) => s.favorite).length },
    { value: UNCATEGORIZED as SoundScope, label: t('sounddeck.uncategorized'), count: sounds.filter((s) => s.category === null).length },
  ]);
  const categories = $derived(
    (sounddeck?.categories ?? []).map((name) => ({ name, count: sounds.filter((s) => s.category === name).length })),
  );
  const sorts = sortOptions();
  const monitorOptions = $derived([
    { value: null as string | null, label: t('sounddeck.followDefault') },
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
    if (added > 0) daemon.toast(t('sounddeck.added', { count: added }));
  }

  function setFavorite(sound: SoundInfo) {
    void daemon.setSoundDeck(daemon.client.call('sounddeck.set_sound', { id: sound.id, patch: { favorite: !sound.favorite } }));
  }

  async function saveCategory() {
    const dialog = categoryDialog;
    const name = dialog?.name.trim();
    if (!dialog || !name) return;
    categoryDialog = null;
    const request =
      dialog.from === null
        ? daemon.client.call('sounddeck.create_category', { name })
        : daemon.client.call('sounddeck.rename_category', { from: dialog.from, to: name });
    if ((await daemon.setSoundDeck(request)) && dialog.from !== null && scope === dialog.from) scope = name;
  }

  async function deleteCategory() {
    const name = deletingCategory;
    deletingCategory = null;
    if (name === null) return;
    if ((await daemon.setSoundDeck(daemon.client.call('sounddeck.delete_category', { name }))) && scope === name) scope = null;
  }

  async function deleteSound() {
    const sound = deletingSound;
    deletingSound = null;
    if (sound) await daemon.setSoundDeck(daemon.client.call('sounddeck.delete', { id: sound.id }));
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
      if (sound && sound.category !== category) void daemon.setSoundDeck(daemon.client.call('sounddeck.move', { id: moving, category }));
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

{#if sounddeck && prefs}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="screen"
    ondragover={(event) => ondragover(event, SCREEN_TARGET)}
    ondragleave={(event) => {
      if (event.currentTarget === event.target || !event.relatedTarget) dropTarget = null;
    }}
    ondrop={(event) => ondrop(event, SCREEN_TARGET)}
  >
    <PageHeader title={t('sounddeck.section')}>
        <Input class="search" compact type="search" bind:value={query} placeholder={t('sounddeck.searchPlaceholder')} aria-label={t('sounddeck.search')}>
          {#snippet icon()}<Search size={14} strokeWidth={1.5} />{/snippet}
        </Input>
        <span class="hud-label">{t('sounddeck.sort')}</span>
        <Dropdown label={t('sounddeck.sort')} options={sorts} value={sort} width="170px" onchange={(value) => {
          sort = value;
          saveSort(value);
        }} />
      {#snippet actions()}
        <button class="btn tool" {@attach tip(t('sounddeck.randomHint'))} aria-label={t('sounddeck.random')} disabled={visible.every((sound) => sound.error)} onclick={() => {
          const pool = visible.filter((sound) => !sound.error);
          const pick = pool[Math.floor(Math.random() * pool.length)];
          if (pick) daemon.playSound(pick.id);
        }}><Shuffle size={13} strokeWidth={1.5} /><span class="label">{t('sounddeck.random')}</span></button>
        <button class="btn tool" {@attach tip(sounddeck.folder)} aria-label={t('sounddeck.openFolder')} onclick={() => daemon.run(daemon.client.call('sounddeck.open_folder'))}><FolderOpen size={13} strokeWidth={1.5} /><span class="label">{t('sounddeck.openFolder')}</span></button>
        <LibraryFolder variant="button" />
        <button class="btn invert" onclick={() => fileInput?.click()}><Plus size={13} strokeWidth={1.5} />{t('sounddeck.addFiles')}</button>
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
        <SettingsLink section="sounddeck" />
      {/snippet}
    </PageHeader>

    {#if !sounddeck.microphone_ready}
      <p class="banner" role="status"><TriangleAlert size={14} strokeWidth={1.5} />{t('sounddeck.cableMissing')}</p>
    {/if}

    <div class="body">
      <aside class="side">
        <section class="categories" aria-label={t('sounddeck.categories')}>
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
          <h2 class="hud-label">{t('sounddeck.categories')}</h2>
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
                  <button class="btn icon ghost small" aria-label={`${t('sounddeck.renameCategory')}: ${category.name}`} {@attach tip(t('sounddeck.renameCategory'))} onclick={() => (categoryDialog = { from: category.name, name: category.name })}><Pencil size={12} strokeWidth={1.5} /></button>
                  <button class="btn icon ghost small" aria-label={`${t('sounddeck.deleteCategory')}: ${category.name}`} {@attach tip(t('sounddeck.deleteCategory'))} onclick={() => (deletingCategory = category.name)}><Trash2 size={12} strokeWidth={1.5} /></button>
                </span>
              </li>
            {/each}
          </ul>
          <button class="new" onclick={() => (categoryDialog = { from: null, name: '' })}><Plus size={13} strokeWidth={1.5} /><span class="hud-label">{t('sounddeck.newCategory')}</span></button>
          {#if problems.length}
            <ul class="problems">
              <li>
                <button class="scope problem" class:active={scope === PROBLEMS} aria-current={scope === PROBLEMS ? 'true' : undefined} {@attach tip(t('sounddeck.unplayableHint'))} onclick={() => (scope = PROBLEMS)}>
                  <TriangleAlert size={13} strokeWidth={1.5} />
                  <span class="label">{t('sounddeck.unplayable')}</span><span class="count value">{problems.length}</span>
                </button>
              </li>
            </ul>
          {/if}
        </section>

        <section class="output" aria-label={t('sounddeck.output')}>
          <h2 class="hud-label">{t('sounddeck.output')}</h2>
          <div class="target">
            <Mic size={14} strokeWidth={1.5} />
            <span class="name" {@attach tip(t('sounddeck.toMicrophoneHint'))}>{t('sounddeck.toMicrophone')}</span>
            <Toggle checked={prefs.to_microphone} label={t('sounddeck.toMicrophone')} color="violet" onchange={(to_microphone) => daemon.setSoundDeckPrefs({ to_microphone })} />
          </div>
          <RulerSlider compact label={t('sounddeck.microphoneVolume')} value={prefs.microphone_volume_pct} min={0} max={200} step={1} defaultValue={100} origin={100} color="violet" disabled={!prefs.to_microphone} format={(value) => `${value}%`} onchange={(microphone_volume_pct) => daemon.setSoundDeckPrefs({ microphone_volume_pct })} />
          <div class="target">
            <Headphones size={14} strokeWidth={1.5} />
            <span class="name" {@attach tip(t('sounddeck.toHeadphonesHint'))}>{t('sounddeck.toHeadphones')}</span>
            <Toggle checked={prefs.to_headphones} label={t('sounddeck.toHeadphones')} color="teal" onchange={(to_headphones) => daemon.setSoundDeckPrefs({ to_headphones })} />
          </div>
          <RulerSlider compact label={t('sounddeck.headphonesVolume')} value={prefs.headphones_volume_pct} min={0} max={200} step={1} defaultValue={70} origin={100} color="teal" disabled={!prefs.to_headphones} format={(value) => `${value}%`} onchange={(headphones_volume_pct) => daemon.setSoundDeckPrefs({ headphones_volume_pct })} />
          <Dropdown label={t('sounddeck.monitorDevice')} options={monitorOptions} value={prefs.monitor_device} disabled={!prefs.to_headphones} onchange={(device) => daemon.setSoundDeck(daemon.client.call('sounddeck.set_monitor_device', { device }))} />
          <label class="target" {@attach tip(t('sounddeck.muteVoiceHint'))}>
            <span class="name">{t('sounddeck.muteVoice')}</span>
            <Toggle checked={prefs.mute_voice_while_playing} label={t('sounddeck.muteVoice')} color="orange" onchange={(mute_voice_while_playing) => daemon.setSoundDeckPrefs({ mute_voice_while_playing })} />
          </label>
        </section>
      </aside>

      <section class="list" aria-label={t('sounddeck.section')}>
        {#if scope === PROBLEMS}
          <div class="problem-head">
            <div>
              <h2 class="hud-label">{t('sounddeck.unplayable')}</h2>
              <p>{t('sounddeck.unplayableBody')}</p>
            </div>
            <button class="btn danger" onclick={() => (confirmProblems = true)}><Trash2 size={13} strokeWidth={1.5} />{t('sounddeck.deleteAllUnplayable')}</button>
          </div>
          <ul class="rows problem-rows" aria-label={t('sounddeck.unplayable')}>
            {#each problems as sound (sound.id)}
              <li class="problem-row">
                <TriangleAlert size={13} strokeWidth={1.5} />
                <span class="title"><b>{sound.name}</b><small>{sound.id.split('/').pop()}</small></span>
                <span class="reason" {@attach tip(sound.error ?? '')}>{sound.error ?? t('sounddeck.unplayableReason')}</span>
                <button class="btn icon ghost small" aria-label={t('sounddeck.deleteSound', { name: sound.name })} {@attach tip(t('sounddeck.deleteSound', { name: sound.name }))} onclick={() => daemon.setSoundDeck(daemon.client.call('sounddeck.delete', { id: sound.id }))}><Trash2 size={13} strokeWidth={1.5} /></button>
              </li>
            {/each}
          </ul>
        {:else if sounds.length === 0}
          <div class="empty">
            <EmptyState label={t('sounddeck.emptyLabel')} title={t('sounddeck.emptyTitle')} accent={t('sounddeck.emptyAccent')}>
              <p>{t('sounddeck.emptyBody')}</p>
              <p class="note">{t('sounddeck.emptyNote', { folder: sounddeck.folder })}</p>
              {#snippet actions()}
                <button class="btn invert" onclick={() => fileInput?.click()}><Plus size={13} strokeWidth={1.5} />{t('sounddeck.addFiles')}</button>
                <button class="btn" onclick={() => daemon.run(daemon.client.call('sounddeck.open_folder'))}><FolderOpen size={13} strokeWidth={1.5} />{t('sounddeck.openFolder')}</button>
                <LibraryFolder variant="button" />
              {/snippet}
            </EmptyState>
          </div>
        {:else}
          <div class="head hud-label" aria-hidden="true">
            <span></span>
            <span>{t('sounddeck.columns.name')}</span>
            <span>{t('sounddeck.columns.category')}</span>
            <span class="right">{t('sounddeck.columns.length')}</span>
            <span>{t('sounddeck.columns.hotkey')}</span>
            <span></span>
          </div>
          <ul class="rows" bind:this={listElement} role="listbox" aria-label={t('sounddeck.section')} onscroll={onrowsscroll}>
            {#each rendered as sound, index (sound.id)}
              {@const playing = playback?.sound_id === sound.id}
            {@const shortcut = shortcuts.get(`sounddeck_play.${sound.id}`) ?? null}
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
                {@attach tip(sound.error ? t('sounddeck.broken', { error: sound.error }) : undefined)}
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
                  aria-label={playing ? t('sounddeck.stop') : t('sounddeck.play', { name: sound.name })}
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
                  {#if shortcut}<kbd>{shortcut}</kbd>{:else}<button class="assign" tabindex="-1" aria-label={`${t('sounddeck.hotkey')}: ${sound.name}`} onclick={(event) => {
                    event.stopPropagation();
                    editing = sound.id;
                  }}><Keyboard size={12} strokeWidth={1.5} /></button>{/if}
                </span>
                <span class="tools">
                  <button class="btn icon ghost small fav" class:on={sound.favorite} tabindex="-1" aria-pressed={sound.favorite} aria-label={sound.favorite ? t('sounddeck.unfavorite') : t('sounddeck.favorite')} {@attach tip(sound.favorite ? t('sounddeck.unfavorite') : t('sounddeck.favorite'))} onclick={(event) => {
                    event.stopPropagation();
                    setFavorite(sound);
                  }}><Star size={13} strokeWidth={1.5} /></button>
                  <button class="btn icon ghost small" tabindex="-1" aria-label={t('sounddeck.edit', { name: sound.name })} {@attach tip(t('sounddeck.edit', { name: sound.name }))} onclick={(event) => {
                    event.stopPropagation();
                    editing = sound.id;
                  }}><Pencil size={13} strokeWidth={1.5} /></button>
                </span>
              </li>
            {:else}
              <li class="none" role="presentation">
                <EmptyState compact role="status" label={query ? t('sounddeck.noMatchesLabel') : t('sounddeck.emptyCategoryLabel')}>
                  <p>{query ? t('sounddeck.noMatches', { query }) : t('sounddeck.noneInCategory')}</p>
                  {#snippet actions()}
                    {#if query}<button class="btn small" onclick={() => (query = '')}>{t('sounddeck.clearSearch')}</button>{/if}
                  {/snippet}
                </EmptyState>
              </li>
            {/each}
          </ul>
        {/if}
        {#each Object.entries(daemon.uploads) as [name, progress] (name)}
          <p class="uploading" role="status">
            <Upload size={13} strokeWidth={1.5} /><span>{t('sounddeck.uploading', { name })}</span>
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
              ? t('sounddeck.dropTo', { name: uploadCategory })
              : t('sounddeck.drop')
            : dropTarget === FAVORITES || dropTarget === UNCATEGORIZED
              ? t('sounddeck.drop')
              : t('sounddeck.dropTo', { name: dropTarget })}
        </span>
      </div>
    {/if}
  </div>
{:else}
  <Loader label={t('daemon.loading')} />
{/if}

{#if editingSound}
  {#key editingSound.id}
    <SoundDialog sound={editingSound} onclose={() => (editing = null)} onrenamed={(id) => (editing = selected = id)} />
  {/key}
{/if}

<Dialog open={confirmProblems} title={t('sounddeck.deleteAllUnplayable')} onclose={() => (confirmProblems = false)}>
  <p>{t('sounddeck.deleteAllUnplayableBody', { count: problems.length })}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (confirmProblems = false)}>{t('common.cancel')}</button>
    <button class="btn danger on" onclick={deleteProblems}>{t('sounddeck.deleteAllUnplayable')}</button>
  {/snippet}
</Dialog>

<Dialog open={categoryDialog !== null} title={categoryDialog?.from ? t('sounddeck.renameCategory') : t('sounddeck.newCategory')} onclose={() => (categoryDialog = null)}>
  {#if categoryDialog}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void saveCategory();
      }}
    >
      <Field label={t('sounddeck.categoryName')}>
        <Input bind:value={categoryDialog.name} maxlength={64} autofocus />
      </Field>
    </form>
  {/if}
  {#snippet actions()}
    <button class="btn" onclick={() => (categoryDialog = null)}>{t('common.cancel')}</button>
    <button class="btn invert" disabled={!categoryDialog?.name.trim()} onclick={saveCategory}>{t('sounddeck.save')}</button>
  {/snippet}
</Dialog>

<Dialog open={deletingCategory !== null} title={t('sounddeck.deleteCategory')} onclose={() => (deletingCategory = null)}>
  <p>{t('sounddeck.deleteCategoryBody', { name: deletingCategory ?? '' })}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (deletingCategory = null)}>{t('common.cancel')}</button>
    <button class="btn danger on" onclick={deleteCategory}>{t('sounddeck.delete')}</button>
  {/snippet}
</Dialog>

<Dialog open={deletingSound !== null} title={t('sounddeck.deleteTitle')} onclose={() => (deletingSound = null)}>
  <p>{t('sounddeck.deleteBody', { name: deletingSound?.name ?? '' })}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (deletingSound = null)}>{t('common.cancel')}</button>
    <button class="btn danger on" onclick={deleteSound}>{t('sounddeck.delete')}</button>
  {/snippet}
</Dialog>

<style>
  .screen {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    container: sounddeck / inline-size;
  }

  /* A narrow window keeps the toolbar on one line: its tools show only their icons. A container
     query, as media queries do not see the UI zoom. */
  @container sounddeck (max-width: 1176px) {
    .tool .label {
      display: none;
    }
  }

  .screen :global(.search) {
    width: 240px;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-6);
    border-bottom: 1px solid var(--line);
    font-size: var(--text-md);
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
    gap: var(--space-1);
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
    gap: var(--space-half);
  }

  .category {
    position: relative;
  }

  .scope {
    display: flex;
    align-items: center;
    gap: var(--space-2);
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
    font-size: var(--text-xs);
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
    gap: var(--space-2);
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
    gap: var(--space-2);
    padding: 14px 16px 16px;
    border-top: 1px solid var(--line);
  }

  .target {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--muted);
    margin-top: 4px;
  }

  .target .name {
    flex: 1;
    min-width: 0;
    font-size: var(--text-md);
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
    column-gap: var(--space-3);
  }

  .head {
    padding: 10px 29px 8px;
    border-bottom: 1px solid var(--line);
    font-size: var(--text-2xs);
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
    gap: var(--space-2);
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
    gap: var(--space-2);
    min-width: 0;
  }

  .title b {
    font-family: var(--font-display);
    font-size: var(--text-lg);
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
    font-size: var(--text-sm);
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
    font-size: var(--text-xs);
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

  .problems {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--line);
  }

  .scope.problem :global(svg) {
    color: var(--app-amber-ink);
  }

  .problem-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-6);
    padding: 16px 24px;
    border-bottom: 1px solid var(--line);
  }

  .problem-head p {
    margin-top: 6px;
    max-width: 560px;
    font-size: var(--text-md);
    color: var(--muted);
  }

  .problem-row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-6);
    border-bottom: 1px solid var(--line);
    color: var(--app-amber-ink);
  }

  .problem-row .title {
    display: grid;
    min-width: 0;
  }

  .problem-row b {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-weight: 400;
    color: var(--ink);
  }

  .problem-row small {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--faint);
  }

  .problem-row .reason {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--text-sm);
    color: var(--muted);
  }

  .none {
    padding: var(--space-3) 0;
  }

  .uploading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 24px;
    border-top: 1px solid var(--line);
    font-size: var(--text-sm);
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

  .dropzone {
    position: absolute;
    inset: 72px 24px 80px 272px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    border: 1px dashed var(--accent);
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    color: var(--accent-ink);
    pointer-events: none;
  }

  .dropzone .hud-label {
    color: var(--accent-ink);
  }
</style>
