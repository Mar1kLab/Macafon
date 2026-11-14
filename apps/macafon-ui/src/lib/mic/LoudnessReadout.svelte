<script lang="ts">
  // Output loudness readout (§6.3.9, MTR-003): short-term LUFS of what apps hear, so the level
  // has a target to aim at. Read from the meter bus a few times a second, not every frame.
  import { onMount } from 'svelte';
  import { meterBus } from '../state/meters';
  import { tip } from '../components/tooltip.svelte';
  import { t } from '../i18n';

  const INTERVAL_MS = 250;
  const SILENT_LUFS = -70;
  let value = $state('—');

  onMount(() => {
    const timer = setInterval(() => {
      const lufs = meterBus.latest.output_lufs_short_term;
      value = lufs === undefined || lufs <= SILENT_LUFS ? '—' : lufs.toFixed(1);
    }, INTERVAL_MS);
    return () => clearInterval(timer);
  });
</script>

<span class="loudness" {@attach tip(t('mic.loudnessHint'))}>
  <span class="hud-label">{t('mic.loudness')}</span>
  <span class="value">{value} LUFS</span>
</span>

<style>
  .loudness {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
  }
</style>
