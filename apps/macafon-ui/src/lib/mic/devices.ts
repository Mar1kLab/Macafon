// Microphone type icons (MIC-001): the daemon's device kind, else a guess from the node name.

import { Bluetooth, Headset, Mic, Usb } from '@lucide/svelte';
import type { Device } from '../ipc/protocol';

export function deviceIcon(device: Pick<Device, 'name' | 'kind'> | undefined) {
  switch (device?.kind) {
    case 'bluetooth':
      return Bluetooth;
    case 'headset':
      return Headset;
    case 'usb':
      return Usb;
    case 'builtin':
      return Mic;
  }
  const name = device?.name ?? '';
  if (name.startsWith('bluez')) return Bluetooth;
  if (/headset|arctis|hyperx|headphone/i.test(name)) return Headset;
  if (name.includes('usb')) return Usb;
  return Mic;
}
