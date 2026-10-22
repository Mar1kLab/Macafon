<script lang="ts">
  import { onDestroy } from 'svelte';
  import Dialog from '../components/Dialog.svelte';
  import RulerSlider from '../components/RulerSlider.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import type { MacaronSettings } from '../ipc/protocol';

  let { id, onclose }: { id: string; onclose: () => void } = $props();
  const device = $derived(daemon.outputControls.find((item) => item.id === id));
  const connected = $derived(daemon.connected && !!device);
  let volume = $state(0);
  let balance = $state(0);
  let busy = $state(false);
  let error = $state('');
  let queued: Partial<MacaronSettings> | null = null;
  let pendingPair: Partial<MacaronSettings> = {};
  let pairTimer = $state<ReturnType<typeof setTimeout> | undefined>();
  let disposed = false;
  onDestroy(() => { disposed = true; clearTimeout(pairTimer); queued = null; });
  const pairKnown = $derived(device?.settings.volume != null && device?.settings.balance != null);
  const filters = $derived([t('outputDevice.fast'), t('outputDevice.ldSlow'), t('outputDevice.ldFast'), t('outputDevice.slow'), t('outputDevice.nos')]);
  const gains = $derived([t('outputDevice.low'), t('outputDevice.mid'), t('outputDevice.high')]);
  const modes = $derived([t('outputDevice.normal'), t('outputDevice.boost')]);
  $effect(() => {
    if (device?.settings.volume != null && !busy && !pairTimer) volume = device.settings.volume;
    if (device?.settings.balance != null && !busy && !pairTimer) balance = device.settings.balance;
  });
  function formatBalance(value: number): string {
    return value === 0 ? t('outputDevice.center') : t(value < 0 ? 'outputDevice.left' : 'outputDevice.right', { value: Math.abs(value) });
  }
  async function set(patch: Partial<MacaronSettings>) {
    if (!connected || disposed) return;
    if (busy) { queued = { ...queued, ...patch }; return; }
    busy = true;
    error = '';
    try {
      daemon.outputControls = await daemon.client.call('output_controls.set', {
        id, settings: { volume: null, balance: null, gain: null, filter: null, output_mode: null, ...patch },
      });
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      queued = null;
    } finally {
      busy = false;
      const next = queued;
      queued = null;
      if (next && connected) void set(next);
    }
  }
  function schedulePair(patch: Partial<MacaronSettings>) {
    pendingPair = { ...pendingPair, ...patch };
    if (pairTimer) return;
    pairTimer = setTimeout(() => {
      pairTimer = undefined;
      const next = pendingPair;
      pendingPair = {};
      void set(next);
    }, 80);
  }
  function setVolume(value: number) { volume = value; schedulePair({ volume }); }
  function setBalance(value: number) { balance = value; schedulePair({ balance }); }
</script>

<Dialog open title={device?.name ?? t('outputDevice.macaron')} {onclose} width="580px">
  <div class="device-panel" aria-busy={busy}>
    {#if !connected}
      <p role="status" class="error">{t('outputDevice.disconnected')}</p>
    {:else}
      <section class="pair" aria-label={t('outputDevice.volumeBalance')}>
        <div class="sliders">
        <RulerSlider value={volume} min={0} max={100} step={1} decimals={0} defaultValue={0}
          label={t('outputDevice.volume')} format={(v) => pairKnown ? `${v}` : '—'} disabled={!pairKnown} color="teal" onchange={setVolume} />
        <RulerSlider value={balance} min={-10} max={10} step={1} decimals={0} defaultValue={0} origin={0}
          label={t('outputDevice.balance')} format={(v) => pairKnown ? formatBalance(v) : '—'} disabled={!pairKnown} color="teal" onchange={setBalance} />
        </div>
      </section>
      {#each [
        { key: 'filter' as const, title: t('outputDevice.filter'), options: filters },
        { key: 'gain' as const, title: t('outputDevice.gain'), options: gains },
        { key: 'output_mode' as const, title: t('outputDevice.mode'), options: modes },
      ] as group (group.key)}
        <fieldset class:filter={group.key === 'filter'}>
          <legend class="hud-label">{group.title}</legend>
          <div class="choices">
            {#each group.options as label, value}
              <button class="btn small" class:selected={device?.settings[group.key] === value}
                aria-pressed={device?.settings[group.key] === value} onclick={() => set({ [group.key]: value })}>{label}</button>
            {/each}
          </div>
        </fieldset>
      {/each}
    {/if}
    {#if error || device?.error}<p class="error" role="alert">{error || device?.error}</p>{/if}
  </div>
</Dialog>

<style>
  .device-panel { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-3); max-height: calc(100vh - 160px); overflow-y: auto; }
  .device-panel > p, .pair, .filter { grid-column: 1 / -1; }
  .sliders { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-4); }
  fieldset, .pair { border: 1px solid var(--line); padding: 12px; display: grid; gap: var(--space-3); min-width: 0; }
  legend { padding: 0 4px; }
  .choices { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .selected { border-color: var(--app-teal); color: var(--app-teal-ink); }
  .error { color: var(--danger); font-size: var(--text-sm); overflow-wrap: anywhere; }
</style>
