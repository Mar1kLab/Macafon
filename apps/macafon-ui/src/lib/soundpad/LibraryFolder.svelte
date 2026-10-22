<script lang="ts">
  // The soundpad library folder (SND-001): any folder on any drive, picked with the system
  // dialog (a typed path in the browser mock), opened in the file manager or reset to the
  // default `Music\Macafon Sounds`.
  import { FolderOpen, FolderSearch, RotateCcw } from '@lucide/svelte';
  import PromptDialog from '../components/PromptDialog.svelte';
  import { pickFolder } from '../ipc/tauri';
  import { daemon } from '../state';
  import { t } from '../i18n';

  /** `button`: only the change button, for the Soundpad screen's toolbar. */
  let { variant = 'full' }: { variant?: 'full' | 'button' } = $props();
  const soundpad = $derived(daemon.soundpad);
  let typing = $state(false);

  function setFolder(folder: string | null) {
    typing = false;
    void daemon.setSoundpad(daemon.client.call('soundpad.set_folder', { folder }));
  }

  async function change() {
    if (!soundpad) return;
    const picked = await daemon.run(pickFolder(soundpad.folder));
    if (picked === undefined) typing = true;
    else if (picked !== null && picked !== soundpad.folder) setFolder(picked);
  }
</script>

{#if soundpad && variant === 'button'}
  <button class="btn tool" title={t('soundpad.changeFolderHint')} aria-label={t('soundpad.changeFolder')} onclick={change}><FolderSearch size={13} strokeWidth={1.5} /><span class="label">{t('soundpad.changeFolder')}</span></button>
{:else if soundpad}
  <div class="folder">
    <code class="path" title={soundpad.folder}>{soundpad.folder}</code>
    {#if soundpad.default_folder}<span class="badge">{t('soundpad.folderDefault')}</span>{/if}
  </div>
  <div class="actions">
    <button class="btn" onclick={change}><FolderSearch size={13} strokeWidth={1.5} />{t('soundpad.changeFolder')}</button>
    <button class="btn" onclick={() => daemon.run(daemon.client.call('soundpad.open_folder'))}><FolderOpen size={13} strokeWidth={1.5} />{t('soundpad.openFolder')}</button>
    {#if !soundpad.default_folder}
      <button class="btn ghost" onclick={() => setFolder(null)}><RotateCcw size={13} strokeWidth={1.5} />{t('soundpad.useDefaultFolder')}</button>
    {/if}
  </div>
{/if}

{#if soundpad}
  <PromptDialog
    open={typing}
    title={t('soundpad.changeFolder')}
    label={t('soundpad.folderPath')}
    initial={soundpad.folder}
    confirm={t('soundpad.useFolder')}
    maxlength={1024}
    onsubmit={setFolder}
    onclose={() => (typing = false)}
  />
{/if}

<style>
  .folder {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: 8px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
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

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
</style>
