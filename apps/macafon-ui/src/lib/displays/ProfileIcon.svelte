<script lang="ts">
  // A display profile's icon (DISPLAY_PROFILE_ICONS) as its Lucide component.
  import { Briefcase, Film, Gamepad2, Laptop, Monitor, MonitorSmartphone, Moon, Tv } from '@lucide/svelte';
  import type { Component } from 'svelte';

  let { icon, size = 16 }: { icon: string; size?: number } = $props();

  const ICONS: Record<string, Component<{ size?: number; strokeWidth?: number }>> = {
    monitor: Monitor,
    monitors: MonitorSmartphone,
    laptop: Laptop,
    gamepad: Gamepad2,
    tv: Tv,
    film: Film,
    briefcase: Briefcase,
    moon: Moon,
  };
  const Icon = $derived(ICONS[icon] ?? Monitor);
</script>

<Icon {size} strokeWidth={1.5} />
