<script lang="ts">
  // Calibration result (CAL-004, §9.2 step 4): SNR before and after and what was set.
  import { formatDb, formatHz, formatValue } from '../dsp/format';
  import { MODULES } from './modules';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import type { CalibrationReport } from '../ipc/protocol';

  let { report }: { report: CalibrationReport } = $props();
  const studio = $derived(daemon.mic?.settings.noise_suppression.engine === 'studio');
</script>

<div class="stats">
  <div>
    <span class="hud-label">{t('calibration.snrBefore')}</span>
    <span class="big">{report.snr_db.toFixed(1)}<small>dB</small></span>
  </div>
  <div>
    <span class="hud-label">{t('calibration.snrAfter')}</span>
    <span class="big accent">{report.snr_after_db.toFixed(1)}<small>dB</small></span>
  </div>
  <div>
    <span class="hud-label">{t('calibration.noiseLevel')}</span>
    <span class="big">{t(`calibration.noise.${report.noise_level}`)}</span>
  </div>
</div>
<span class="hud-label">{t('calibration.changed')}</span>
<ul class="changes">
  <li style:--c="var(--app-{MODULES.gain.color}-ink)"><span class="dot"></span>{t(MODULES.gain.title)}<span class="value">{formatDb(report.gain_db)}</span></li>
  <li style:--c="var(--app-{MODULES.high_pass.color}-ink)"><span class="dot"></span>{t(MODULES.high_pass.title)}<span class="value">{formatHz(report.high_pass_freq_hz)}</span></li>
  <li style:--c="var(--app-{MODULES.noise_suppression.color}-ink)"><span class="dot"></span>{t(MODULES.noise_suppression.title)}<span class="value">{formatValue(report.noise_intensity_pct, 'percent')}</span></li>
  <li style:--c="var(--app-{MODULES.expander.color}-ink)"><span class="dot"></span>{t(MODULES.expander.title)} · {t('params.threshold')}<span class="value">{formatValue(report.expander_threshold_db, 'dbfs')}</span></li>
</ul>
{#if report.suggest_studio && !studio}
  <p class="note studio">
    {t('calibration.suggestStudio')}
    <button class="btn small" onclick={() => daemon.edit((s) => (s.noise_suppression.engine = 'studio'))}>{t('calibration.useStudio')}</button>
  </p>
{/if}

<style>
  .note {
    margin-top: 12px;
    font-size: var(--text-md);
    color: var(--muted);
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-3);
    margin-bottom: 20px;
  }

  .stats > div {
    display: grid;
    gap: var(--space-1);
    padding: 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .big {
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 600;
    letter-spacing: -0.03em;
    color: var(--ink-strong);
  }

  .big small {
    margin-left: 4px;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--muted);
  }

  .big.accent {
    color: var(--app-teal-ink);
  }

  .changes {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }

  .changes li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 0;
    border-bottom: 1px solid var(--line);
  }

  .changes .value {
    margin-left: auto;
  }

  .dot {
    width: 8px;
    height: 8px;
    background: var(--c);
  }

  .studio {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
</style>
