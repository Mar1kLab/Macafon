<script lang="ts">
  // Settings → Updates (§8.6): Macafon itself, then the tools SoundDeck downloads with. Checks and
  // downloads start only from these buttons or the opt-in automatic check (UPD-001, UPD-002).
  import { Download, ExternalLink, RefreshCw, RotateCw, Trash2, X } from '@lucide/svelte';
  import Toggle from '../components/Toggle.svelte';
  import Dialog from '../components/Dialog.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { tip } from '../components/tooltip.svelte';
  import { formatBytes } from '../dictation/format';
  import type { ComponentUpdate, UpdateComponent } from '../ipc/protocol';

  const updates = $derived(daemon.updates);
  const checking = $derived(!!updates && [updates.app, ...updates.tools].some((entry) => entry.stage === 'checking'));
  let removing = $state<UpdateComponent | null>(null);

  const NAMES: Record<UpdateComponent, string> = { app: 'Macafon', yt_dlp: 'yt-dlp', ffmpeg: 'ffmpeg' };

  function busy(entry: ComponentUpdate): boolean {
    return entry.stage === 'downloading' || entry.stage === 'verifying' || entry.stage === 'installing';
  }

  function lastChecked(unix: number | null): string {
    if (unix === null) return t('updates.never');
    return new Date(unix * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
  }

  function stageLabel(entry: ComponentUpdate): string {
    switch (entry.stage) {
      case 'downloading':
        return entry.progress_pct === null ? t('updates.downloading') : t('updates.downloadingPct', { percent: Math.round(entry.progress_pct) });
      case 'verifying':
        return t('updates.verifying');
      case 'installing':
        return entry.component === 'app' ? t('updates.launching') : t('updates.installing');
      default:
        return '';
    }
  }

  function status(entry: ComponentUpdate): { label: string; tone: 'new' | 'ok' | 'off' } | null {
    if (entry.component !== 'app' && entry.source === 'missing') return { label: t('updates.notInstalled'), tone: 'off' };
    if (entry.update_available) return { label: t('updates.available'), tone: 'new' };
    if (entry.latest_version && entry.installed_version === entry.latest_version) return { label: t('updates.upToDate'), tone: 'ok' };
    return null;
  }

  function sourceLabel(entry: ComponentUpdate): string | null {
    switch (entry.source) {
      case 'managed':
        return t('updates.sourceManaged');
      case 'custom':
        return t('updates.sourceCustom');
      case 'path':
        return t('updates.sourcePath');
      default:
        return null;
    }
  }

  function size(entry: ComponentUpdate): string {
    return entry.download_bytes ? ` · ${formatBytes(entry.download_bytes)}` : '';
  }
</script>

{#snippet versions(entry: ComponentUpdate)}
  <span class="versions value">
    {entry.installed_version ?? '—'}
    {#if entry.update_available && entry.latest_version}<span class="arrow" aria-hidden="true">→</span><span class="latest">{entry.latest_version}</span>{/if}
  </span>
{/snippet}

{#snippet progress(entry: ComponentUpdate)}
  <div class="progress-row">
    <span
      class="progress"
      role="progressbar"
      aria-label={t('updates.progressOf', { name: NAMES[entry.component] })}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={entry.progress_pct === null ? undefined : Math.round(entry.progress_pct)}
    >
      <span class="fill" class:indeterminate={entry.progress_pct === null} style:width="{entry.progress_pct ?? 100}%"></span>
    </span>
    <span class="stage">{stageLabel(entry)}</span>
    {#if entry.stage !== 'installing'}
      <button class="btn icon ghost" aria-label={t('updates.cancel', { name: NAMES[entry.component] })} {@attach tip(t('updates.cancel', { name: NAMES[entry.component] }))} onclick={() => daemon.cancelUpdate(entry.component)}><X size={14} strokeWidth={1.5} /></button>
    {/if}
  </div>
{/snippet}

{#snippet card(entry: ComponentUpdate)}
  {@const state = status(entry)}
  {@const source = sourceLabel(entry)}
  <div class="card" class:new={entry.update_available}>
    <div class="head">
      <span class="title">{NAMES[entry.component]}</span>
      {@render versions(entry)}
      {#if source}<span class="badge">{source}</span>{/if}
      {#if state}<span class="badge {state.tone}">{state.label}</span>{/if}
      <span class="spacer"></span>
      {#if entry.release_url && !(entry.component === 'app' && !entry.installable && entry.update_available)}
        <button class="btn small ghost" onclick={() => daemon.openRelease(entry.component)}><ExternalLink size={12} strokeWidth={1.5} />{t('updates.releaseNotes')}</button>
      {/if}
    </div>
    {#if entry.path && entry.component !== 'app'}<code class="path" {@attach tip(entry.path)}>{entry.path}</code>{/if}

    {#if busy(entry)}
      {@render progress(entry)}
    {:else if entry.component === 'app'}
      {#if entry.stage === 'ready'}
        <div class="actions">
          <p class="note">{t('updates.readyHint', { version: entry.latest_version ?? '' })}</p>
          <button class="btn invert" onclick={() => daemon.installUpdate('app')}><RotateCw size={13} strokeWidth={1.5} />{t('updates.installRestart')}</button>
        </div>
      {:else if entry.update_available}
        <div class="actions">
          {#if entry.installable}
            <button class="btn invert" onclick={() => daemon.installUpdate('app')}><Download size={13} strokeWidth={1.5} />{t('updates.download')}{size(entry)}</button>
          {:else}
            <p class="note">{t('updates.packageManager')}</p>
            <button class="btn" onclick={() => daemon.openRelease('app')}><ExternalLink size={13} strokeWidth={1.5} />{t('updates.openRelease')}</button>
          {/if}
        </div>
      {/if}
    {:else if entry.installable}
      <div class="actions">
        {#if entry.source === 'managed'}
          {#if entry.update_available}
            <button class="btn invert" onclick={() => daemon.installUpdate(entry.component)}><Download size={13} strokeWidth={1.5} />{t('updates.update')}{size(entry)}</button>
          {/if}
          <button class="btn ghost" onclick={() => (removing = entry.component)}><Trash2 size={13} strokeWidth={1.5} />{t('updates.remove')}</button>
        {:else}
          {#if entry.source !== 'missing'}<p class="note">{t('updates.externalHint')}</p>{/if}
          <button class={entry.source === 'missing' ? 'btn invert' : 'btn'} onclick={() => daemon.installUpdate(entry.component)}>
            <Download size={13} strokeWidth={1.5} />{entry.source === 'missing' ? t('updates.install') : t('updates.installOwn')}{size(entry)}
          </button>
        {/if}
      </div>
    {:else if entry.source === 'missing'}
      <p class="note">{t('updates.linuxTools')}</p>
    {/if}

    {#if entry.error && entry.stage === 'failed'}<p class="error" role="alert">{entry.error}</p>{/if}
  </div>
{/snippet}

{#if updates}
  <div class="row">
    <div>
      <p class="name">{t('updates.automatic')}</p>
      <p class="hint">{t('updates.automaticHint')}</p>
    </div>
    <Toggle checked={updates.check_automatically} label={t('updates.automatic')} onchange={(enabled: boolean) => daemon.setCheckUpdatesAutomatically(enabled)} />
  </div>
  <div class="row">
    <div>
      <p class="name">{t('updates.lastChecked')}</p>
      <p class="hint">{lastChecked(updates.last_checked_unix)}</p>
    </div>
    <button class="btn" disabled={checking} onclick={() => daemon.checkUpdates()}>
      <span class="spin" class:on={checking}><RefreshCw size={13} strokeWidth={1.5} /></span>{checking ? t('updates.checking') : t('updates.checkNow')}
    </button>
  </div>
  <div class="row column">
    <p class="hud-label group">{t('updates.app')}</p>
    {@render card(updates.app)}
  </div>
  <div class="row column">
    <div>
      <p class="hud-label group">{t('updates.dependencies')}</p>
      <p class="hint">{t('updates.dependenciesHint')}</p>
    </div>
    {#each updates.tools as tool (tool.component)}
      {@render card(tool)}
    {/each}
  </div>
{/if}

<Dialog open={removing !== null} title={t('updates.removeTitle', { name: removing ? NAMES[removing] : '' })} onclose={() => (removing = null)}>
  <p class="confirm">{t('updates.removeConfirm')}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (removing = null)}>{t('common.cancel')}</button>
    <button
      class="btn danger on"
      onclick={() => {
        if (removing) daemon.removeTool(removing);
        removing = null;
      }}>{t('updates.remove')}</button
    >
  {/snippet}
</Dialog>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-6);
    padding: 14px 24px;
  }

  .row + .row {
    border-top: 1px solid var(--line);
  }

  .row.column {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-3);
  }

  .name {
    color: var(--ink);
  }

  .hint {
    font-size: var(--text-md);
    color: var(--muted);
  }

  .group {
    color: var(--muted);
  }

  .card {
    display: grid;
    gap: var(--space-2);
    padding: 10px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .card.new {
    border-color: var(--accent);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .title {
    font-family: var(--font-display);
    font-size: var(--text-lg);
    color: var(--ink-strong);
  }

  .versions {
    display: inline-flex;
    gap: var(--space-1);
    color: var(--muted);
  }

  .arrow {
    color: var(--faint);
  }

  .latest {
    color: var(--accent-ink);
  }

  .badge.new {
    border-color: var(--accent);
    color: var(--accent-ink);
  }

  .badge.off {
    color: var(--faint);
  }

  .spacer {
    flex: 1;
  }

  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--faint);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .note {
    flex: 1;
    min-width: 200px;
    font-size: var(--text-sm);
    color: var(--muted);
  }

  .error {
    font-size: var(--text-sm);
    color: var(--danger);
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
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
    overflow: hidden;
  }

  .progress .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    transition: width var(--dur-reveal) linear;
  }

  .progress .fill.indeterminate {
    opacity: 0.5;
  }

  .stage {
    min-width: 140px;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    color: var(--ink);
  }

  .spin {
    display: inline-flex;
  }

  .spin.on {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spin.on {
      animation: none;
    }
  }

  .confirm {
    color: var(--muted);
  }
</style>
