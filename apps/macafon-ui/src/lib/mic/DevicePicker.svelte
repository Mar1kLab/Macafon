<script lang="ts">
  // Microphone picker (MIC-001, MIC-005): the list refreshes when opened; the active input
  // shows its live level. The daemon meters only the linked microphone, so other entries
  // have no meter yet (MIC-002 needs per-device metering in macafond).
  import { onMount } from 'svelte';
  import { Bluetooth, Headset, Mic, Usb } from '@lucide/svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import { daemon } from '../state';
  import { cssVar, fitCanvas, onFrame } from '../state/frame';
  import { Ballistics, meterBus } from '../state/meters';
  import { t } from '../i18n';
  import type { MicState } from '../ipc/protocol';

  let { mic }: { mic: MicState } = $props();
  let canvas: HTMLCanvasElement | undefined = $state();

  const options = $derived([
    ...daemon.devices.map((device) => ({ value: device.name as string | null, label: device.description })),
    { value: null, label: t('mic.noDevice') },
  ]);
  const current = $derived(daemon.devices.find((device) => device.name === mic.device));

  /** Device type icon from the PipeWire node name (USB / Bluetooth / headset / built-in). */
  function kind(name: string | null | undefined) {
    if (!name) return Mic;
    if (name.startsWith('bluez')) return Bluetooth;
    if (/headset|arctis|hyperx|headphone/i.test(name)) return Headset;
    if (name.includes('usb')) return Usb;
    return Mic;
  }

  onMount(() => {
    const level = new Ballistics();
    let colors: [string, string] | null = null;
    return onFrame((now) => {
      if (!canvas) return;
      colors ??= [cssVar(canvas, '--app-blue-ink'), cssVar(canvas, '--line')];
      level.update(meterBus.latest.raw_peak_dbfs, now);
      const context = fitCanvas(canvas);
      if (!context) return;
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      context.clearRect(0, 0, w, h);
      const lit = Math.round(Math.min(Math.max((level.level + 60) / 60, 0), 1) * 5);
      for (let i = 0; i < 5; i++) {
        context.fillStyle = i < lit ? colors[0] : colors[1];
        context.fillRect(i * 4, 0, 3, h);
      }
    });
  });
</script>

<div class="picker" class:disconnected={!mic.input_connected}>
  <Dropdown
    label={t('mic.device')}
    {options}
    value={mic.device}
    width="260px"
    onopen={() => daemon.refreshDevices()}
    onchange={(device) => daemon.setMic(daemon.client.call('mic.set_device', { device }))}
  >
    {#snippet button()}
      {@const Icon = kind(mic.device)}
      <Icon size={15} strokeWidth={1.5} />
      <span class="name">{current?.description ?? mic.device ?? t('mic.noDevice')}</span>
      {#if mic.device && !mic.input_connected}
        <span class="badge off">{t('mic.disconnected')}</span>
      {:else}
        <canvas bind:this={canvas} class="mini" aria-hidden="true"></canvas>
      {/if}
    {/snippet}
    {#snippet option(item, selected)}
      {@const Icon = kind(item.value)}
      <Icon size={14} strokeWidth={1.5} />
      <span>{item.label}</span>
      {#if selected && item.value && !mic.input_connected}<span class="badge off">{t('mic.disconnected')}</span>{/if}
    {/snippet}
  </Dropdown>
</div>

<style>
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mini {
    width: 19px;
    height: 10px;
    margin-left: auto;
    flex: none;
  }

  .off {
    margin-left: auto;
    color: var(--danger);
  }

  .disconnected :global(.trigger) {
    border-color: color-mix(in srgb, var(--danger) 50%, transparent);
  }
</style>
