<script lang="ts">
  // Doctor (PRD §9.6): self-checks in plain words, one-click fixes where Macafon can fix
  // something itself, and a copyable report without device or app names.
  import { CircleAlert, CircleCheck, CircleX, Info } from '@lucide/svelte';
  import RefreshCw from '../icons/animated/RefreshCw.svelte';
  import { daemon } from '../state';
  import { t, type MessageKey } from '../i18n';
  import type { DoctorCheck, DoctorCheckId, DoctorReport } from '../ipc/protocol';
  import { openVbCablePage } from '../ipc/tauri';

  let { onnavigate, onmakedefault }: { onnavigate: (screen: 'microphone' | 'mixer') => void; onmakedefault: () => void } =
    $props();

  let report = $state.raw<DoctorReport | null>(null);
  let running = $state(false);
  let started = false;

  async function run() {
    running = true;
    const result = await daemon.run(daemon.client.call('doctor.run'));
    running = false;
    if (result) report = result;
  }

  $effect(() => {
    if (daemon.connected && !started) {
      started = true;
      void run();
    }
  });

  const ICONS = { ok: CircleCheck, info: Info, warning: CircleAlert, error: CircleX } as const;

  function text(check: DoctorCheck): string {
    const key = `doctor.${check.id}.${check.status}` as MessageKey;
    const values: Record<string, string> = Object.fromEntries(check.values.map((value, index) => [index, value]));
    return t(key, { ...values, list: check.values.join(', ') });
  }

  type Fix = { label: MessageKey; action: () => void };
  function fix(check: DoctorCheck): Fix | null {
    const id: DoctorCheckId = check.id;
    if (id === 'microphone_input' && check.status !== 'ok') {
      return { label: 'doctor.fixMicrophone', action: () => onnavigate('microphone') };
    }
    if (id === 'microphone_used' && check.status === 'info') return { label: 'doctor.fixDefault', action: onmakedefault };
    if (check.fixable) {
      return {
        label: id === 'cable_default_output' ? 'doctor.fixCableOutput' : 'doctor.run',
        action: () => void daemon.run(daemon.client.call('doctor.fix', { check: id })).then(run),
      };
    }
    if (id === 'vb_cable' && check.status === 'error') {
      return { label: 'doctor.fixCable', action: () => void openVbCablePage().catch((error) => daemon.fail(error)) };
    }
    if (id === 'mixer' && check.status === 'info') {
      return {
        label: 'doctor.fixMixer',
        action: () => void daemon.setMixer(daemon.client.call('mixer.set_enabled', { enabled: true })).then(run),
      };
    }
    return null;
  }

  /** Versions, statuses and numbers only: names of devices and apps stay private. */
  async function copy() {
    if (!report) return;
    const numeric = new Set<DoctorCheckId>(['pipewire', 'session_manager', 'processing', 'xruns', 'cable_latency']);
    const lines = [
      `Macafon ${report.daemon_version} (UI ${__APP_VERSION__})`,
      daemon.windows
        ? 'Windows · VB-CABLE'
        : `PipeWire ${report.pipewire_version ?? '?'} · ${report.session_manager?.replace(/\s*\(.*\)/, '') ?? 'no session manager'}`,
      `Quantum ${report.quantum ?? '?'} @ ${report.sample_rate_hz ?? '?'} Hz`,
      `Mixer ${daemon.mixer?.enabled ? 'on' : 'off'}, ${daemon.mixer?.channels.length ?? 0} channels`,
      ...report.checks.map((check) => {
        const detail = numeric.has(check.id)
          ? check.values.filter(Boolean).join(' / ')
          : check.values.length
            ? `${check.values.length} item(s)`
            : '';
        return `${check.status.padEnd(7)} ${check.id}${detail ? `: ${detail}` : ''}`;
      }),
    ];
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      daemon.toast(t('doctor.copied'));
    } catch (error) {
      daemon.fail(error);
    }
  }
</script>

<div class="doctor">
  <div class="actions">
    <button class="btn" disabled={running} onclick={run}><RefreshCw size={13} strokeWidth={1.5} />{t('doctor.run')}</button>
    <button class="btn" disabled={!report} onclick={copy}>{t('doctor.copy')}</button>
  </div>
  {#if report}
    <ul>
      {#each report.checks as check (check.id)}
        {@const Icon = ICONS[check.status]}
        {@const action = fix(check)}
        <li class={check.status}>
          <Icon size={15} strokeWidth={1.5} />
          <div>
            <p class="name">{t(`doctor.${check.id}.title` as MessageKey)}</p>
            <p class="hint">{text(check)}</p>
          </div>
          {#if action}<button class="btn small" onclick={action.action}>{t(action.label)}</button>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .doctor {
    display: grid;
    gap: var(--space-3);
    padding: 14px 24px;
  }

  .actions {
    display: flex;
    gap: var(--space-2);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: var(--space-1);
  }

  li {
    display: grid;
    grid-template-columns: 16px 1fr auto;
    gap: var(--space-3);
    align-items: center;
    padding: 8px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  li.ok {
    color: var(--app-teal-ink);
  }

  li.info {
    color: var(--accent-ink);
  }

  li.warning {
    color: var(--app-amber-ink);
  }

  li.error {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
  }

  .name {
    color: var(--ink);
  }

  .hint {
    font-size: var(--text-md);
    color: var(--muted);
  }
</style>
