<script lang="ts">
  // The SoundDeck library folder (SND-001): any folder on any drive, picked with the system
  // dialog (a typed path in the browser mock), opened in the file manager or reset to the
  // default `Music\Macafon Sounds`.
  import { FolderOpen, FolderSearch } from '@lucide/svelte';
  import RotateCcw from '../icons/animated/RotateCcw.svelte';
  import PromptDialog from '../components/PromptDialog.svelte';
  import { pickFolder } from '../ipc/tauri';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';

  /** `button`: only the change button, for the SoundDeck screen's toolbar. */
  let { variant = 'full' }: { variant?: 'full' | 'button' } = $props();
  const sounddeck = $derived(daemon.sounddeck);
  let typing = $state(false);

  function setFolder(folder: string | null) {
    typing = false;
    void daemon.setSoundDeck(daemon.client.call('sounddeck.set_folder', { folder }));
  }

  async function change() {
    if (!sounddeck) return;
    const picked = await daemon.run(pickFolder(sounddeck.folder));
    if (picked === undefined) typing = true;
    else if (picked !== null && picked !== sounddeck.folder) setFolder(picked);
  }
</script>

{#if sounddeck && variant === 'button'}
  <button class="btn tool" {@attach tip(t('sounddeck.changeFolderHint'))} aria-label={t('sounddeck.changeFolder')} onclick={change}><FolderSearch size={13} strokeWidth={1.5} /><span class="label">{t('sounddeck.changeFolder')}</span></button>
{:else if sounddeck}
  <div class="folder">
    <code class="path" {@attach tip(sounddeck.folder)}>{sounddeck.folder}</code>
    {#if sounddeck.default_folder}<span class="badge">{t('sounddeck.folderDefault')}</span>{/if}
  </div>
  <div class="actions">
    <button class="btn" onclick={change}><FolderSearch size={13} strokeWidth={1.5} />{t('sounddeck.changeFolder')}</button>
    <button class="btn" onclick={() => daemon.run(daemon.client.call('sounddeck.open_folder'))}><FolderOpen size={13} strokeWidth={1.5} />{t('sounddeck.openFolder')}</button>
    {#if !sounddeck.default_folder}
      <button class="btn ghost" onclick={() => setFolder(null)}><RotateCcw size={13} strokeWidth={1.5} />{t('sounddeck.useDefaultFolder')}</button>
    {/if}
  </div>
{/if}

{#if sounddeck}
  <PromptDialog
    open={typing}
    title={t('sounddeck.changeFolder')}
    label={t('sounddeck.folderPath')}
    initial={sounddeck.folder}
    confirm={t('sounddeck.useFolder')}
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
