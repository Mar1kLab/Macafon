<script lang="ts">
  // Modal dialog: native <dialog> for focus trapping and Escape, ticks on the corners.
  import type { Snippet } from 'svelte';
  import { X } from '@lucide/svelte';
  import Ticks from './Ticks.svelte';
  import { t } from '../i18n';

  interface Props {
    open: boolean;
    title: string;
    onclose: () => void;
    children: Snippet;
    actions?: Snippet;
    width?: string;
  }

  let { open, title, onclose, children, actions, width = '440px' }: Props = $props();
  let dialog: HTMLDialogElement | undefined = $state();

  $effect(() => {
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  });
</script>

<dialog
  bind:this={dialog}
  style:width
  aria-label={title}
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
>
  {#if open}
    <Ticks />
    <header>
      <h2 class="hud-label">{title}</h2>
      <button class="btn icon ghost" aria-label={t('common.close')} onclick={onclose}>
        <X size={16} strokeWidth={1.5} />
      </button>
    </header>
    <div class="body">{@render children()}</div>
    {#if actions}
      <footer>{@render actions()}</footer>
    {/if}
  {/if}
</dialog>

<style>
  dialog {
    overflow: visible;
    padding: 0;
    border: 1px solid var(--line-strong);
    background: var(--bg-raised);
    color: var(--ink);
    max-width: calc(100vw - 48px);
  }

  dialog::backdrop {
    background: color-mix(in srgb, var(--bg-sunken) 78%, transparent);
  }

  dialog[open] {
    animation: appear var(--dur-card) ease-out;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 12px 24px;
    border-bottom: 1px solid var(--line);
  }

  h2 {
    color: var(--ink);
  }

  .body {
    padding: 24px;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 0 24px 24px;
  }

  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
