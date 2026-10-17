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
  function schedulePair() {
    if (!pairKnown || pairTimer) return;
    pairTimer = setTimeout(() => { pairTimer = undefined; void set({ volume, balance }); }, 80);
  }
  function setVolume(value: number) { volume = value; schedulePair(); }
  function setBalance(value: number) { balance = value; schedulePair(); }
</script>

<Dialog open title={device?.name ?? t('outputDevice.macaron')} {onclose} width="580px">
  <div class="device-panel">
    <p class="hint">{t('outputDevice.scope')}</p>
    <p class="notice">{t('outputDevice.writeOnly')}</p>
    {#if !connected}
      <p role="status" class="error">{t('outputDevice.disconnected')}</p>
    {:else}
      <fieldset class="pair">
        <legend class="hud-label">{t('outputDevice.volumeBalance')}</legend>
        {#if !pairKnown}<span class="badge">{t('outputDevice.unknown')}</span>{/if}
        {#if device?.live_volume}<span class="badge">{t('outputDevice.liveVolume')}</span>{/if}
        <div class="sliders">
        <RulerSlider value={volume} min={0} max={100} step={1} decimals={0} defaultValue={0}
          label={t('outputDevice.volume')} format={(v) => `${v}`} color="teal" onchange={setVolume} />
        <RulerSlider value={balance} min={-10} max={10} step={1} decimals={0} defaultValue={0} origin={0}
          label={t('outputDevice.balance')} format={formatBalance} color="teal" onchange={setBalance} />
        </div>
        {#if !pairKnown}
          <p class="hint">{t('outputDevice.pairHint')}</p>
          <button class="btn small" disabled={busy} onclick={() => set({ volume, balance })}>{t('outputDevice.applyPair')}</button>
        {/if}
      </fieldset>
      {#each [
        { key: 'filter' as const, title: t('outputDevice.filter'), options: filters },
        { key: 'gain' as const, title: t('outputDevice.gain'), options: gains },
        { key: 'output_mode' as const, title: t('outputDevice.mode'), options: modes },
      ] as group (group.key)}
        <fieldset class:filter={group.key === 'filter'}>
          <legend class="hud-label">{group.title}</legend>
          {#if device?.settings[group.key] == null}<span class="badge">{t('outputDevice.unknown')}</span>{/if}
          <div class="choices">
            {#each group.options as label, value}
              <button class="btn small" class:selected={device?.settings[group.key] === value}
                aria-pressed={device?.settings[group.key] === value} onclick={() => set({ [group.key]: value })}>{label}</button>
            {/each}
          </div>
        </fieldset>
      {/each}
      <p class="hint" role="status">{busy ? t('outputDevice.applying') : t('outputDevice.session')}</p>
    {/if}
    {#if error || device?.error}<p class="error" role="alert">{error || device?.error}</p>{/if}
  </div>
</Dialog>

<style>
  .device-panel { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; max-height: calc(100vh - 160px); overflow-y: auto; }
  .device-panel > p, .pair, .filter { grid-column: 1 / -1; }
  .sliders { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  .hint { color: var(--muted); font-size: 12px; line-height: 1.5; }
  .notice { border-left: 2px solid var(--app-amber); padding-left: 12px; color: var(--muted); font-size: 12px; line-height: 1.5; }
  fieldset { border: 1px solid var(--line); padding: 12px; display: grid; gap: 12px; min-width: 0; }
  legend { padding: 0 4px; }
  .badge { justify-self: start; }
  .choices { display: flex; flex-wrap: wrap; gap: 4px; }
  .selected { border-color: var(--app-teal); color: var(--app-teal-ink); }
  .error { color: var(--danger); font-size: 12px; overflow-wrap: anywhere; }
</style>
