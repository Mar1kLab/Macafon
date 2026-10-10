<script lang="ts">
  // First steps on the Microphone screen until dismissed (US-2, US-3, US-5, MIC-015). The
  // dismissal is a UI preference, kept in the WebView's local storage.
  import { Check } from '@lucide/svelte';
  import { t } from '../i18n';

  let { calibrated, oncalibrate }: { calibrated: boolean; oncalibrate: () => void } = $props();

  const KEY = 'macafon.tips.microphone';
  let dismissed = $state(localStorage.getItem(KEY) === 'dismissed');

  function dismiss() {
    dismissed = true;
    localStorage.setItem(KEY, 'dismissed');
  }
</script>

{#if !dismissed}
  <section class="tip" aria-label={t('mic.tipTitle')}>
    <span class="hud-label title">{t('mic.tipTitle')}</span>
    <ol>
      <li class:done={calibrated}>
        <span class="n">{#if calibrated}<Check size={12} strokeWidth={2} />{:else}1{/if}</span>
        <button class="link" onclick={oncalibrate}>{t('mic.tipStep1')}</button>
      </li>
      <li><span class="n">2</span>{t('mic.tipStep2')}</li>
      <li><span class="n">3</span><span>{t('mic.tipStep3')}<small>{t('mic.tipStep3Note')}</small></span></li>
    </ol>
    <button class="btn small" onclick={dismiss}>{t('mic.tipDismiss')}</button>
  </section>
{/if}

<style>
  .tip {
    position: relative;
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: start;
    gap: 16px;
    margin-bottom: 16px;
    padding: 12px 16px;
    border: 1px solid var(--line);
    border-left: 2px solid var(--accent);
    background: var(--bg-raised);
  }

  .title {
    color: var(--ink);
    padding-top: 2px;
  }

  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 20px;
  }

  li {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    font-size: 13px;
    color: var(--ink);
  }

  li.done {
    color: var(--muted);
  }

  .n {
    flex: none;
    width: 20px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--line-strong);
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--accent-ink);
  }

  .done .n {
    border-color: var(--app-teal);
    color: var(--app-teal-ink);
  }

  .link {
    text-align: left;
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--line-strong);
    text-underline-offset: 3px;
  }

  .link:hover {
    text-decoration-color: var(--accent-ink);
  }

  small {
    display: block;
    margin-top: 4px;
    font-size: 12px;
    color: var(--muted);
  }
</style>
