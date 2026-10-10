<script lang="ts">
  // Toasts in the bottom-right corner with a HUD title (design system §6).
  import { X } from '@lucide/svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each daemon.toasts as toast (toast.id)}
    <div class="toast" class:danger={toast.tone === 'danger'}>
      <div>
        <p class="hud-label title">{toast.title}</p>
        {#if toast.message}<p class="message">{toast.message}</p>{/if}
      </div>
      <button class="btn icon ghost" aria-label={t('common.close')} onclick={() => daemon.dismiss(toast.id)}>
        <X size={14} strokeWidth={1.5} />
      </button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 20px;
    bottom: 96px;
    z-index: 50;
    display: grid;
    gap: 8px;
    width: 320px;
  }

  .toast {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 6px 10px 14px;
    background: var(--bg-raised);
    border: 1px solid var(--line);
    border-left: 2px solid var(--accent);
    animation: slide var(--dur-card) ease-out;
  }

  .toast.danger {
    border-left-color: var(--danger);
  }

  .title {
    color: var(--ink);
  }

  .message {
    margin-top: 2px;
    font-size: 13px;
    color: var(--muted);
  }

  @keyframes slide {
    from {
      opacity: 0;
      transform: translateX(12px);
    }
  }
</style>
