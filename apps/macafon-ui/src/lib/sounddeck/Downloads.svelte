<script lang="ts">
  // Link downloads under the library (SND-009): progress and cancel while running, the error
  // and dismiss when one failed. Finished ones become sounds and leave the list.
  import { Download, TriangleAlert, X } from '@lucide/svelte';
  import type { SoundDeckDownload } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';

  const downloads = $derived((daemon.sounddeck?.downloads ?? []).filter((download) => download.state !== 'done'));

  function label(download: SoundDeckDownload): string {
    const name = download.title ?? download.url;
    switch (download.state) {
      case 'queued':
        return t('sounddeck.link.queued', { name });
      case 'converting':
        return t('sounddeck.link.converting', { name });
      case 'failed':
        return t('sounddeck.link.failed', { name });
      case 'cancelled':
        return t('sounddeck.link.cancelled', { name });
      default:
        return t('sounddeck.link.downloading', { name });
    }
  }

  const running = (download: SoundDeckDownload) =>
    download.state === 'queued' || download.state === 'downloading' || download.state === 'converting';
</script>

{#each downloads as download (download.id)}
  <div class="download" class:failed={download.state === 'failed'} role="status">
    {#if download.state === 'failed'}
      <TriangleAlert size={13} strokeWidth={1.5} />
    {:else}
      <Download size={13} strokeWidth={1.5} />
    {/if}
    <span class="text">
      <span class="name" {@attach tip(download.url)}>{label(download)}</span>
      {#if download.error}<span class="error">{download.error}</span>{/if}
    </span>
    {#if running(download)}
      <span
        class="bar"
        class:indeterminate={download.progress_pct === null}
        role="progressbar"
        aria-label={label(download)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={download.progress_pct ?? undefined}
      >
        <span style:width="{download.progress_pct ?? 100}%"></span>
      </span>
      <button class="btn icon ghost small" aria-label={t('sounddeck.link.cancel')} {@attach tip(t('sounddeck.link.cancel'))} onclick={() => daemon.cancelDownload(download.id)}><X size={13} strokeWidth={1.5} /></button>
    {:else}
      <button class="btn icon ghost small" aria-label={t('sounddeck.link.dismiss')} {@attach tip(t('sounddeck.link.dismiss'))} onclick={() => daemon.dismissDownload(download.id)}><X size={13} strokeWidth={1.5} /></button>
    {/if}
  </div>
{/each}

<style>
  .download {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 24px;
    border-top: 1px solid var(--line);
    font-size: var(--text-sm);
    color: var(--muted);
  }

  .download.failed {
    color: var(--danger);
  }

  .text {
    display: flex;
    flex: 1;
    min-width: 0;
    gap: var(--space-2);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .error {
    flex-shrink: 0;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }

  .bar {
    flex: 1;
    max-width: 240px;
    height: 4px;
    background: var(--bg-sunken);
  }

  .bar > span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width var(--dur) var(--ease);
  }

  .bar.indeterminate > span {
    opacity: 0.4;
  }
</style>
