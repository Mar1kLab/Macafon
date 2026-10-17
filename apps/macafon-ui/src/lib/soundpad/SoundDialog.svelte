<script lang="ts">
  // One sound: name (the file name), category (its folder), volume, favorite and hotkeys
  // (SND-004, SND-005); deleting removes the file from the library folder.
  import { Play, Trash2 } from '@lucide/svelte';
  import Dialog from '../components/Dialog.svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import RulerSlider from '../components/RulerSlider.svelte';
  import Toggle from '../components/Toggle.svelte';
  import ShortcutKey from './ShortcutKey.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { formatDuration } from './format';
  import type { SoundInfo } from '../ipc/protocol';

  interface Props {
    sound: SoundInfo;
    onclose: () => void;
    /** The sound got a new id (renamed or moved). */
    onrenamed: (id: string) => void;
  }

  let { sound, onclose, onrenamed }: Props = $props();

  // svelte-ignore state_referenced_locally
  let name = $state(sound.name);
  let confirmDelete = $state(false);
  const categories = $derived([
    { value: null as string | null, label: t('soundpad.uncategorized') },
    ...(daemon.soundpad?.categories ?? []).map((category) => ({ value: category as string | null, label: category })),
  ]);
  const action = $derived(`soundpad_play.${sound.id}`);

  async function rename() {
    const next = name.trim();
    if (!next || next === sound.name) return;
    const state = await daemon.run(daemon.client.call('soundpad.rename', { id: sound.id, name: next }));
    if (!state) {
      name = sound.name;
      return;
    }
    daemon.soundpad = state;
    follow(state, `${folder(sound.category)}${next}${extension(sound.id)}`);
  }

  const folder = (category: string | null) => (category ? `${category}/` : '');
  const extension = (id: string) => /\.[^./]+$/.exec(id)?.[0] ?? '';

  /** Keep editing the sound under its new id; close if the daemon picked another name. */
  function follow(state: { sounds: SoundInfo[] }, id: string) {
    if (state.sounds.some((item) => item.id === id)) onrenamed(id);
    else onclose();
  }

  async function move(category: string | null) {
    if (category === sound.category) return;
    const state = await daemon.run(daemon.client.call('soundpad.move', { id: sound.id, category }));
    if (!state) return;
    daemon.soundpad = state;
    follow(state, `${folder(category)}${sound.id.split('/').pop()}`);
  }

  function edit(patch: { volume_pct?: number; favorite?: boolean }) {
    if (daemon.soundpad) {
      daemon.soundpad = {
        ...daemon.soundpad,
        sounds: daemon.soundpad.sounds.map((item) => (item.id === sound.id ? { ...item, ...patch } : item)),
      };
    }
    void daemon.setSoundpad(daemon.client.call('soundpad.set_sound', { id: sound.id, patch }));
  }

  async function remove() {
    confirmDelete = false;
    if (await daemon.setSoundpad(daemon.client.call('soundpad.delete', { id: sound.id }))) onclose();
  }
</script>

<Dialog open={!confirmDelete} title={t('soundpad.soundTitle')} {onclose} width="520px">
  <div class="form">
    <label class="field">
      <span class="hud-label">{t('soundpad.name')}</span>
      <input
        class="input"
        bind:value={name}
        maxlength={120}
        onblur={rename}
        onkeydown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault();
            void rename();
          }
        }}
      />
    </label>
    <div class="field">
      <span class="hud-label">{t('soundpad.category')}</span>
      <Dropdown label={t('soundpad.category')} options={categories} value={sound.category} onchange={move} />
    </div>
    <div class="field">
      <RulerSlider
        label={t('soundpad.volume')}
        value={sound.volume_pct}
        min={0}
        max={200}
        step={1}
        defaultValue={100}
        origin={100}
        color="violet"
        format={(value) => `${value}%`}
        onchange={(volume_pct) => edit({ volume_pct })}
      />
    </div>
    <div class="field inline">
      <span class="hud-label">{t('soundpad.favorite')}</span>
      <Toggle checked={sound.favorite} label={t('soundpad.favorite')} color="amber" onchange={(favorite) => edit({ favorite })} />
    </div>
    <div class="field">
      <span class="hud-label">{t('soundpad.hotkeys')}</span>
      <div class="keys">
        <ShortcutKey {action} label={sound.name} />
        <ShortcutKey {action} secondary label={sound.name} />
      </div>
    </div>
    <p class="meta value">
      {sound.id} · {formatDuration(sound.duration_ms, true)} · {t('soundpad.plays', { count: sound.play_count })}
    </p>
  </div>
  {#snippet actions()}
    <button class="btn danger" onclick={() => (confirmDelete = true)}><Trash2 size={13} strokeWidth={1.5} />{t('soundpad.delete')}</button>
    <span class="spacer"></span>
    <button class="btn" disabled={sound.error !== null} onclick={() => daemon.playSound(sound.id)}><Play size={13} strokeWidth={1.5} />{t('soundpad.preview')}</button>
    <button class="btn invert" onclick={onclose}>{t('common.close')}</button>
  {/snippet}
</Dialog>

<Dialog open={confirmDelete} title={t('soundpad.deleteTitle')} onclose={() => (confirmDelete = false)}>
  <p>{t('soundpad.deleteBody', { name: sound.name })}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (confirmDelete = false)}>{t('common.cancel')}</button>
    <button class="btn danger on" onclick={remove}>{t('soundpad.delete')}</button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: grid;
    gap: 16px;
  }

  .field {
    display: grid;
    gap: 8px;
  }

  .field.inline {
    grid-template-columns: 1fr auto;
    align-items: center;
  }

  .keys {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    align-items: start;
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

  .meta {
    color: var(--muted);
    font-size: 11px;
    overflow-wrap: anywhere;
  }

  .spacer {
    flex: 1;
  }

  .danger:not(.on) {
    color: var(--danger);
  }
</style>
