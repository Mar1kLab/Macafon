<script lang="ts">
  // Calibration (§6.5): silence → sentence, then the result (CAL-004) or the failure with a
  // retry (CAL-005, CAL-006).
  import Dialog from '../components/Dialog.svelte';
  import CalibrationProgress from './CalibrationProgress.svelte';
  import CalibrationResult from './CalibrationResult.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';

  let { open, onclose, onlisten }: { open: boolean; onclose: () => void; onlisten: () => void } = $props();

  let started = $state(false);
  const cal = $derived(daemon.calibration);
  const phase = $derived(started ? cal.state : 'idle');

  $effect(() => {
    if (!open) started = false;
  });

  async function start() {
    started = true;
    const result = await daemon.run(daemon.client.call('calibration.start'));
    if (result) daemon.calibration = result;
    else started = false;
  }

  function close() {
    if (cal.state === 'measuring') void daemon.run(daemon.client.call('calibration.cancel'));
    onclose();
  }
</script>

<Dialog {open} title={t('calibration.title')} onclose={close} width="520px">
  {#if phase === 'idle'}
    <p class="lead">{t('calibration.intro')}</p>
    <p class="note">{t('mic.doubleProcessing')}</p>
  {:else if phase === 'measuring' && cal.state === 'measuring'}
    <CalibrationProgress state={cal} />
  {:else if phase === 'done' && cal.state === 'done'}
    <CalibrationResult report={cal.report} />
  {:else if phase === 'failed' && cal.state === 'failed'}
    <p class="failed">{t(`calibration.failed.${cal.reason}`)}</p>
  {/if}
  {#snippet actions()}
    {#if phase === 'idle'}
      <button class="btn" onclick={close}>{t('calibration.cancel')}</button>
      <button class="btn invert" onclick={start}>{t('calibration.start')}</button>
    {:else if phase === 'measuring'}
      <button class="btn" onclick={close}>{t('calibration.cancel')}</button>
    {:else if phase === 'done'}
      <button class="btn" onclick={onclose}>{t('calibration.fineTune')}</button>
      <button class="btn invert" onclick={onlisten}>{t('calibration.listen')}</button>
    {:else}
      <button class="btn" onclick={onclose}>{t('calibration.close')}</button>
      <button class="btn invert" onclick={start}>{t('calibration.retry')}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .lead {
    font-size: 14px;
    color: var(--ink);
  }

  .note {
    margin-top: 12px;
    font-size: 13px;
    color: var(--muted);
  }

  .failed {
    color: var(--danger);
  }
</style>
