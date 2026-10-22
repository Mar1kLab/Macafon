<script lang="ts">
  // Microphone picker (MIC-001, MIC-002, MIC-005): the list refreshes when opened and every
  // microphone shows its live level while it is open, so the one that hears you lights up.
  import { onDestroy } from 'svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import DeviceLevel from './DeviceLevel.svelte';
  import { deviceIcon } from './devices';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import type { MicState } from '../ipc/protocol';

  let { mic }: { mic: MicState } = $props();
  let unwatch: (() => void) | null = null;

  const options = $derived([
    ...daemon.devices.map((device) => ({ value: device.name as string | null, label: device.description })),
    { value: null, label: t('mic.noDevice') },
  ]);
  const current = $derived(daemon.devices.find((device) => device.name === mic.device));
  const byName = (name: string | null) => daemon.devices.find((device) => device.name === name);

  function opened() {
    unwatch ??= daemon.watchDevices();
  }

  function closed() {
    unwatch?.();
    unwatch = null;
  }

  onDestroy(closed);
</script>

<div class="picker" class:disconnected={!mic.input_connected}>
  <Dropdown
    label={t('mic.device')}
    {options}
    value={mic.device}
    width="260px"
    onopen={opened}
    onclose={closed}
    onchange={(device) => daemon.setMic(daemon.client.call('mic.set_device', { device }), true)}
  >
    {#snippet button()}
      {@const Icon = deviceIcon(current ?? (mic.device ? { name: mic.device } : undefined))}
      <Icon size={15} strokeWidth={1.5} />
      <span class="name">{current?.description ?? mic.device ?? t('mic.noDevice')}</span>
      {#if mic.device && !mic.input_connected}
        <span class="badge off">{t('mic.disconnected')}</span>
      {:else}
        <span class="level"><DeviceLevel name={mic.active_device ?? null} active={mic.input_connected} /></span>
      {/if}
    {/snippet}
    {#snippet option(item, selected)}
      {@const Icon = deviceIcon(byName(item.value) ?? undefined)}
      <Icon size={14} strokeWidth={1.5} />
      <span class="name">{item.label}</span>
      {#if selected && item.value && !mic.input_connected}
        <span class="badge off">{t('mic.disconnected')}</span>
      {:else if item.value}
        <span class="level"><DeviceLevel name={item.value} active={item.value === mic.active_device} /></span>
      {/if}
    {/snippet}
  </Dropdown>
</div>

<style>
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .level {
    display: inline-flex;
    margin-left: auto;
  }

  .off {
    margin-left: auto;
    color: var(--danger);
  }

  .disconnected :global(.trigger) {
    border-color: color-mix(in srgb, var(--danger) 50%, transparent);
  }
</style>
