// Channel icons (CH-002); ids come from `CHANNEL_ICONS` of macafon-config.

import { Bell, Film, Gamepad2, Globe, Headphones, MessageCircle, Music, Speaker } from '@lucide/svelte';

export const CHANNEL_ICON = {
  speaker: Speaker,
  message: MessageCircle,
  music: Music,
  gamepad: Gamepad2,
  film: Film,
  bell: Bell,
  globe: Globe,
  headphones: Headphones,
} as const;

export function channelIcon(id: string) {
  return CHANNEL_ICON[id as keyof typeof CHANNEL_ICON] ?? Speaker;
}
