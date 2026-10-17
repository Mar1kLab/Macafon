// Plausible local desktop fixtures shared by the mock subsystems.
import { CHANNEL_TEMPLATES } from '../dsp';
import type { AppState, ChannelSettings, ChannelState, Device, MixerState } from '../protocol';
import { channelLatency } from './helpers';

export const DEVICES: Device[] = [
  { name: 'alsa_input.usb-Blue_Microphones_Yeti_Stereo_Microphone-00.analog-stereo', description: 'Yeti Stereo Microphone', kind: 'usb' },
  { name: 'alsa_input.usb-SteelSeries_Arctis_Nova_7-00.mono-fallback', description: 'Arctis Nova 7 Headset', kind: 'headset' },
  { name: 'alsa_input.pci-0000_00_1f.3.analog-stereo', description: 'Built-in Audio Analog Stereo', kind: 'builtin' },
];

export function channel(id: string, name: string, color: string, icon: string, settings: ChannelSettings): ChannelState {
  return { id, name, color, icon, enabled: true, settings: structuredClone(settings), latency_samples: channelLatency(settings) };
}

function app(key: string, name: string, channel: string, active: boolean, streams = 1): AppState {
  const icon = key.startsWith('bin:') ? key.slice(4) : null;
  return { key, name, icon, channel, active, streams, managed: true, movable: true, volume_pct: key === 'bin:discord' ? 80 : 100 };
}

/** Stand-ins for icon theme files (APP-003); other apps keep their letter. */
export const MOCK_ICONS: Record<string, string> = {
  firefox:
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><circle cx="10" cy="10" r="9" fill="#e8573f"/><circle cx="10" cy="10" r="5" fill="#f2a516"/></svg>',
  discord:
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><rect x="1" y="4" width="18" height="12" rx="5" fill="#8b5cf6"/><circle cx="7" cy="10" r="1.6" fill="#fff"/><circle cx="13" cy="10" r="1.6" fill="#fff"/></svg>',
  cider:
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><rect width="20" height="20" rx="4" fill="#e0368f"/><path d="M8 14V6l6-1v8" stroke="#fff" stroke-width="1.6" fill="none"/></svg>',
  'telegram-desktop':
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><circle cx="10" cy="10" r="9" fill="#2f7bf5"/><path d="M5 10l9-4-2 9-3-3z" fill="#fff"/></svg>',
};

/** Plausible desktop: a browser and a game in Main, Discord in Voice, Cider in Music. */
export function initialMixer(enabled: boolean): MixerState {
  return {
    enabled,
    default_channel: 'main',
    master_volume_pct: 100,
    master_muted: false,
    auto_classify: true,
    channels: [
      channel('main', 'Main', 'blue', 'speaker', CHANNEL_TEMPLATES.main),
      channel('voice', 'Voice', 'violet', 'message', CHANNEL_TEMPLATES.voice),
      channel('music', 'Music', 'pink', 'music', CHANNEL_TEMPLATES.music),
    ],
    apps: enabled
      ? [
          app('bin:firefox', 'Firefox', 'main', true, 2),
          app('name:Cyberpunk 2077', 'Cyberpunk 2077', 'main', true),
          app('system:sounds', 'System sounds', 'main', false),
          app('bin:discord', 'Discord', 'voice', true, 2),
          app('bin:telegram-desktop', 'Telegram', 'voice', false, 0),
          app('bin:cider', 'Cider', 'music', true),
        ]
      : [],
    outputs: [
      { name: 'alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo', description: 'Arctis Nova 7', kind: 'headphones' },
      { name: 'alsa_output.pci-0000_00_1f.3.analog-stereo', description: 'Built-in Speakers', kind: 'speakers' },
    ],
    output_priority: [],
    output: enabled ? 'alsa_output.usb-SteelSeries_Arctis_Nova_7-00.analog-stereo' : null,
  };
}
