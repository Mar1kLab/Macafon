<script lang="ts">
  // Add a sound from a page link (SND-009): the daemon downloads it with the user's yt-dlp.
  // Without yt-dlp the dialog shows how to install it instead of the form.
  import { Copy, RefreshCw } from '@lucide/svelte';
  import Dialog from '../components/Dialog.svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import Field from '../components/Field.svelte';
  import Input from '../components/Input.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { installCommand, isLink } from './download';

  let { open, category, onclose }: { open: boolean; category: string | null; onclose: () => void } = $props();

  const sounddeck = $derived(daemon.sounddeck);
  const downloader = $derived(sounddeck?.downloader);
  let url = $state('');
  let target = $state<string | null>(null);
  let touched = $state(false);
  let busy = $state(false);
  let checking = $state(false);

  // Every opening starts empty, in the category the screen shows.
  $effect.pre(() => {
    if (open) {
      url = '';
      target = category;
      touched = false;
    }
  });

  const valid = $derived(isLink(url));
  const categories = $derived([
    { value: null, label: t('sounddeck.uncategorized') },
    ...(sounddeck?.categories ?? []).map((name) => ({ value: name as string | null, label: name })),
  ]);
  const command = $derived(installCommand(daemon.windows));

  async function submit() {
    touched = true;
    if (!valid || busy) return;
    busy = true;
    const started = await daemon.downloadSound(url.trim(), target);
    busy = false;
    if (started) onclose();
  }

  async function check() {
    checking = true;
    await daemon.checkDownloader();
    checking = false;
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

<Dialog {open} title={t('sounddeck.link.title')} {onclose} width="480px">
  {#if downloader && !downloader.available}
    <div class="missing" role="status">
      <p class="name">{t('sounddeck.link.missingTitle')}</p>
      <p class="hint">{t('sounddeck.link.missingHint')}</p>
      <div class="command">
        <code>{command}</code>
        <button class="btn icon ghost small" aria-label={t('sounddeck.link.copy')} onclick={copy}><Copy size={13} strokeWidth={1.5} /></button>
      </div>
    </div>
  {:else}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <Field label={t('sounddeck.link.url')} hint={touched && !valid ? t('sounddeck.link.invalidUrl') : t('sounddeck.link.hint')}>
        <Input
          bind:value={url}
          type="url"
          inputmode="url"
          placeholder={t('sounddeck.link.urlPlaceholder')}
          maxlength={2048}
          aria-invalid={touched && !valid}
          onblur={() => (touched = url.trim() !== '')}
          autofocus
        />
      </Field>
      <Field label={t('sounddeck.link.category')} group>
        <Dropdown label={t('sounddeck.link.category')} options={categories} value={target} onchange={(value) => (target = value)} />
      </Field>
      {#if downloader && !downloader.ffmpeg}
        <p class="note">{t('sounddeck.link.noFfmpeg')}</p>
      {/if}
    </form>
  {/if}
  {#snippet actions()}
    <button class="btn" onclick={onclose}>{t('common.cancel')}</button>
    {#if downloader && !downloader.available}
      <button class="btn invert" disabled={checking} onclick={check}><RefreshCw size={13} strokeWidth={1.5} />{t('sounddeck.link.checkAgain')}</button>
    {:else}
      <button class="btn invert" disabled={busy || (touched && !valid)} onclick={submit}>{t('sounddeck.link.download')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  form :global([aria-invalid='true']) {
    color: var(--danger);
  }

  .missing {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .name {
    font-size: var(--text-md);
    color: var(--ink-strong);
  }

  .hint,
  .note {
    font-size: var(--text-sm);
    color: var(--muted);
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
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--ink);
  }
</style>
