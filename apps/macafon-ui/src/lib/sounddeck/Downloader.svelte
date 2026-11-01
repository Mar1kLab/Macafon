<script lang="ts">
  // Settings → SoundDeck: the yt-dlp that link downloads use (SND-009). Found on PATH unless the
  // user sets the program path; ffmpeg is optional and only reported.
  import { Copy, FileCog, RefreshCw } from '@lucide/svelte';
  import RotateCcw from '../icons/animated/RotateCcw.svelte';
  import PromptDialog from '../components/PromptDialog.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';
  import { installCommand } from './download';

  const downloader = $derived(daemon.sounddeck?.downloader);
  let typing = $state(false);
  let checking = $state(false);
  const command = $derived(installCommand(daemon.windows));

  async function check() {
    checking = true;
    await daemon.checkDownloader();
    checking = false;
  }

  function setPath(path: string | null) {
    typing = false;
    void daemon.setDownloaderPath(path?.trim() || null);
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      daemon.toast(t('sounddeck.link.copied'));
    } catch (error) {
      daemon.fail(error);
    }
  }
</script>

{#if downloader}
  <div class="tool">
    <span class="state" class:missing={!downloader.available}>
      {downloader.available ? t('sounddeck.link.found', { version: downloader.version ?? '' }) : t('sounddeck.link.notFound')}
    </span>
    {#if downloader.path}<code class="path" {@attach tip(downloader.path)}>{downloader.path}</code>{:else}<span class="path"></span>{/if}
    {#if downloader.available && !downloader.configured_path}<span class="badge">{t('sounddeck.link.fromPath')}</span>{/if}
    {#if downloader.available}
      <span class="badge" class:off={!downloader.ffmpeg} {@attach tip(downloader.ffmpeg ? '' : t('sounddeck.link.noFfmpeg'))}>
        {downloader.ffmpeg ? t('sounddeck.link.ffmpegFound') : t('sounddeck.link.ffmpegMissing')}
      </span>
    {/if}
  </div>
  {#if !downloader.available}
    <div class="command">
      <code>{command}</code>
      <button class="btn icon ghost small" aria-label={t('sounddeck.link.copy')} {@attach tip(t('sounddeck.link.copy'))} onclick={copy}><Copy size={13} strokeWidth={1.5} /></button>
    </div>
  {/if}
  <div class="actions">
    <button class="btn" disabled={checking} onclick={check}><RefreshCw size={13} strokeWidth={1.5} />{t('sounddeck.link.checkAgain')}</button>
    <button class="btn" onclick={() => (typing = true)}><FileCog size={13} strokeWidth={1.5} />{t('sounddeck.link.setPath')}</button>
    {#if downloader.configured_path}
      <button class="btn ghost" onclick={() => setPath(null)}><RotateCcw size={13} strokeWidth={1.5} />{t('sounddeck.link.usePath')}</button>
    {/if}
  </div>

  <PromptDialog
    open={typing}
    title={t('sounddeck.link.setPath')}
    label={t('sounddeck.link.programPath')}
    initial={downloader.configured_path ?? downloader.path ?? ''}
    confirm={t('sounddeck.link.usePathConfirm')}
    maxlength={1024}
    onsubmit={setPath}
    onclose={() => (typing = false)}
  />
{/if}

<style>
  .tool {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: 8px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    font-size: var(--text-sm);
  }

  .state {
    flex-shrink: 0;
    color: var(--ink);
  }

  .state.missing {
    color: var(--danger);
  }

  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    color: var(--muted);
  }

  .badge.off {
    color: var(--faint);
  }

  .command {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 6px 6px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  code {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .command code {
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    white-space: nowrap;
    color: var(--ink);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
</style>
