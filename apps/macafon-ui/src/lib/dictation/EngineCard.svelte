<script lang="ts">
  // 01 ENGINE (PRD §9.9.1): the engine, one row per manifest model with its download state, and
  // the models folder (STT-013, STT-022). Downloads start only from these buttons.
  import { Download, FileInput, FolderSearch, RotateCcw, Trash2, X } from '@lucide/svelte';
  import Dialog from '../components/Dialog.svelte';
  import PromptDialog from '../components/PromptDialog.svelte';
  import Segmented from '../components/Segmented.svelte';
  import Card from './Card.svelte';
  import { pickFolder } from '../ipc/tauri';
  import type { DictationEngine, DictationModel, DictationState } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { formatBytes, percent } from './format';
  import { tip } from '../components/tooltip.svelte';

  let { dictation }: { dictation: DictationState } = $props();

  const prefs = $derived(dictation.prefs);
  const storage = $derived(dictation.storage);
  const models = $derived(dictation.models.filter((model) => model.engine === prefs.engine));
  const nothingYet = $derived(dictation.models.every((model) => model.status === 'missing'));
  const recommended = $derived(dictation.models.find((model) => model.recommended));
  const engines: { value: DictationEngine; label: string }[] = [
    { value: 'whisper', label: t('dictation.engine.whisper') },
    { value: 'parakeet', label: t('dictation.engine.parakeet') },
  ];

  let deleting = $state<DictationModel | null>(null);
  let importing = $state(false);
  let typingFolder = $state(false);

  const call = daemon.setDictation.bind(daemon);

  function download(model: DictationModel) {
    void call(daemon.client.call('dictation.download', { model: model.id }));
  }

  function choose(model: DictationModel) {
    if (model.status === 'ready' && prefs.model !== model.id) daemon.setDictationPrefs({ model: model.id });
  }

  function setFolder(folder: string | null) {
    typingFolder = false;
    void call(daemon.client.call('dictation.set_models_dir', { folder }));
  }

  async function changeFolder() {
    const picked = await daemon.run(pickFolder(storage.folder));
    if (picked === undefined) typingFolder = true;
    else if (picked !== null && picked !== storage.folder) setFolder(picked);
  }
</script>

{#snippet rating(label: string, value: number)}
  <span class="rating" {@attach tip(t('dictation.engine.ratingValue', { label, value }))}>
    <span class="hud-label">{label}</span>
    <span class="steps" aria-hidden="true">
      {#each [1, 2, 3, 4, 5] as step (step)}<i class:lit={step <= value}></i>{/each}
    </span>
    <span class="visually-hidden">{t('dictation.engine.ratingValue', { label, value })}</span>
  </span>
{/snippet}

<Card number="01" title={t('dictation.engine.title')} hint={t('dictation.engine.hint')} color="blue" class="engine">
  <Segmented wrap label={t('dictation.engine.label')} options={engines} value={prefs.engine} onchange={(engine) => daemon.setDictationPrefs({ engine })} />

  {#if nothingYet && recommended}
    <div class="empty">
      <div>
        <p class="name">{t('dictation.engine.emptyTitle')} <span class="badge">{formatBytes(recommended.size_bytes)}</span></p>
        <p class="hint">{t('dictation.engine.emptyBody')}</p>
      </div>
      <button class="btn invert" onclick={() => download(recommended)}><Download size={13} strokeWidth={1.5} />{t('dictation.engine.download')}</button>
    </div>
  {/if}

  <ul class="models" role="radiogroup" aria-label={t('dictation.engine.label')}>
    {#each models as model (model.id)}
      {@const selected = prefs.model === model.id}
      {@const loaded = dictation.loaded_model === model.id}
      <li class="model" class:selected>
        <button
          class="pick"
          role="radio"
          aria-checked={selected}
          aria-label={t('dictation.engine.use', { name: model.name })}
          disabled={model.status !== 'ready'}
          onclick={() => choose(model)}
        ><span class="radio" aria-hidden="true"></span></button>
        <span class="title">{model.name}</span>
        <span class="size">{formatBytes(model.size_bytes)}</span>
        {@render rating(t('dictation.engine.quality'), model.quality)}
        {@render rating(t('dictation.engine.speed'), model.speed)}
        <span class="state">
          {#if model.status === 'downloading'}
            {@const done = percent(model.downloaded_bytes, model.size_bytes)}
            <span class="progress" role="progressbar" aria-label={model.name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={done} aria-valuetext="{done} %{model.bytes_per_sec ? `, ${formatBytes(model.bytes_per_sec)} per second` : ''}">
              <span class="fill" style:width="{done}%"></span>
            </span>
            <span class="value">{t('dictation.engine.progress', { percent: done })}</span>
            {#if model.bytes_per_sec}<span class="speed">{t('dictation.engine.speedValue', { speed: formatBytes(model.bytes_per_sec) })}</span>{/if}
            <button class="btn icon ghost" aria-label={t('dictation.engine.cancel', { name: model.name })} {@attach tip(t('dictation.engine.cancel', { name: model.name }))} onclick={() => call(daemon.client.call('dictation.cancel_download', { model: model.id }))}><X size={14} strokeWidth={1.5} /></button>
          {:else if model.status === 'ready'}
            <span class="badge" class:active={loaded}>{loaded ? t('dictation.engine.active') : t('dictation.engine.ready')}</span>
            <button class="btn icon ghost" aria-label="{t('dictation.engine.delete')} {model.name}" {@attach tip(t('dictation.engine.delete'))} onclick={() => (deleting = model)}><Trash2 size={14} strokeWidth={1.5} /></button>
          {:else if model.status === 'error'}
            <span class="error" {@attach tip(model.error ?? '')}>{model.error}</span>
            <button class="btn small" onclick={() => download(model)}>{t('dictation.engine.retry')}</button>
          {:else}
            <button class="btn small" onclick={() => download(model)}><Download size={12} strokeWidth={1.5} />{t('dictation.engine.get')}</button>
          {/if}
        </span>
      </li>
    {/each}
  </ul>

  <div class="storage">
    <span class="hud-label">{t('dictation.storage.label')}</span>
    {#if storage.moving}
      {@const done = percent(storage.moving.done_bytes, storage.moving.total_bytes)}
      <span class="progress wide" role="progressbar" aria-label={t('dictation.storage.moving', { percent: done })} aria-valuemin={0} aria-valuemax={100} aria-valuenow={done}><span class="fill" style:width="{done}%"></span></span>
      <span class="value">{t('dictation.storage.moving', { percent: done })}</span>
      <button class="btn icon ghost" aria-label={t('dictation.storage.cancelMove')} {@attach tip(t('dictation.storage.cancelMove'))} onclick={() => call(daemon.client.call('dictation.cancel_move'))}><X size={14} strokeWidth={1.5} /></button>
    {:else}
      <code class="path" {@attach tip(storage.folder)}>{storage.folder}</code>
      {#if storage.free_bytes !== null}<span class="free">{t('dictation.storage.free', { size: formatBytes(storage.free_bytes) })}</span>{/if}
      <button class="btn small" onclick={changeFolder}><FolderSearch size={12} strokeWidth={1.5} />{t('dictation.storage.change')}</button>
      {#if !storage.default_folder}
        <button class="btn small ghost" onclick={() => setFolder(null)}><RotateCcw size={12} strokeWidth={1.5} />{t('dictation.storage.reset')}</button>
      {/if}
    {/if}
  </div>
  {#if storage.rotational}<p class="note">{t('dictation.storage.rotational')}</p>{/if}

  {#snippet footer()}
    <span class="foot">
      <span>{t('dictation.engine.source')} · {t('dictation.engine.used', { size: formatBytes(storage.used_bytes) })}</span>
      <button class="link" {@attach tip(t('dictation.engine.importHint'))} onclick={() => (importing = true)}><FileInput size={12} strokeWidth={1.5} />{t('dictation.engine.import')}</button>
    </span>
  {/snippet}
</Card>

<Dialog open={deleting !== null} title={deleting ? t('dictation.engine.deleteConfirm', { name: deleting.name }) : ''} onclose={() => (deleting = null)}>
  {#if deleting}<p>{t('dictation.engine.deleteBody', { size: formatBytes(deleting.size_bytes) })}</p>{/if}
  {#snippet actions()}
    <button class="btn ghost" onclick={() => (deleting = null)}>{t('common.cancel')}</button>
    <button class="btn danger" onclick={() => {
      const model = deleting;
      deleting = null;
      if (model) void call(daemon.client.call('dictation.delete_model', { model: model.id }));
    }}>{t('dictation.engine.delete')}</button>
  {/snippet}
</Dialog>

<PromptDialog
  open={importing}
  title={t('dictation.engine.import')}
  label={t('dictation.engine.importPrompt')}
  confirm={t('dictation.engine.import')}
  maxlength={1024}
  onsubmit={(path) => {
    importing = false;
    void call(daemon.client.call('dictation.import_model', { path }));
  }}
  onclose={() => (importing = false)}
/>

<PromptDialog
  open={typingFolder}
  title={t('dictation.storage.pickTitle')}
  label={t('dictation.storage.pickTitle')}
  initial={storage.folder}
  confirm={t('dictation.storage.change')}
  maxlength={1024}
  onsubmit={setFolder}
  onclose={() => (typingFolder = false)}
/>

<style>
  .empty {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: 12px 14px;
    border: 1px solid var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-sunken));
  }

  .empty > div {
    flex: 1;
  }

  .name {
    font-family: var(--font-display);
    font-weight: 600;
    font-size: var(--text-lg);
    color: var(--ink-strong);
  }

  .hint,
  .note {
    font-size: var(--text-md);
    color: var(--muted);
  }

  .note {
    color: var(--app-amber-ink);
  }

  .models {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }

  .model {
    display: grid;
    grid-template-columns: 22px minmax(64px, 1fr) 64px auto auto minmax(190px, 1.4fr);
    align-items: center;
    gap: var(--space-3);
    min-height: 40px;
    padding: 0 8px 0 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    transition: border-color var(--dur) var(--ease);
  }

  .model.selected {
    border-color: var(--line-strong);
  }

  .pick {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
  }

  .pick:disabled {
    cursor: default;
    opacity: 0.35;
  }

  .radio {
    width: 10px;
    height: 10px;
    border: 1px solid var(--line-strong);
  }

  .selected .radio {
    background: var(--accent);
    border-color: var(--accent);
  }

  .title {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--ink);
    white-space: nowrap;
  }

  .size,
  .value,
  .speed,
  .free {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--muted);
    white-space: nowrap;
  }

  .rating {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .rating .hud-label {
    font-size: var(--text-2xs);
    color: var(--faint);
  }

  .steps {
    display: flex;
    gap: var(--space-half);
  }

  .steps i {
    width: 6px;
    height: 10px;
    background: var(--line);
  }

  .steps i.lit {
    background: var(--accent-ink);
  }

  .state {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    min-width: 0;
  }

  .badge.active {
    color: var(--app-teal-ink);
    border-color: color-mix(in srgb, var(--app-teal) 50%, transparent);
  }

  .error {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
    color: var(--danger);
  }

  .progress {
    position: relative;
    flex: 1;
    min-width: 60px;
    height: 6px;
    background: var(--bg-raised);
    border: 1px solid var(--line);
    /* Ruler ticks every 10 %. */
    background-image: repeating-linear-gradient(90deg, transparent 0 calc(10% - 1px), var(--line) calc(10% - 1px) 10%);
  }

  .progress .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    transition: width var(--dur-reveal) linear;
  }

  .storage {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--ink);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .foot > span {
    flex: 1;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font: inherit;
    color: var(--muted);
    text-transform: uppercase;
  }

  .link:hover {
    color: var(--ink);
  }
</style>
