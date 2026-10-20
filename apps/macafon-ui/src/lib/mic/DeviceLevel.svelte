<script lang="ts">
  // Five-segment level of one microphone (MIC-002). Watched devices report their own peak;
  // otherwise the linked microphone shows its raw level and the others stay dark.
  import { onMount } from 'svelte';
  import { cssVar, drawEveryFrame } from '../state/frame';
  import { Ballistics, SILENCE_DB, deviceMeterBus, meterBus } from '../state/meters';

  let {
    name,
    active = false,
    segments = 5,
    color = 'blue',
  }: { name: string | null; active?: boolean; segments?: number; color?: string } = $props();
  let canvas: HTMLCanvasElement | undefined = $state();

  onMount(() => {
    const level = new Ballistics();
    const colors = [cssVar(canvas!, `--app-${color}-ink`), cssVar(canvas!, '--line')] as const;
    return drawEveryFrame(canvas!, (surface, now) => {
      const watched = name === null ? undefined : deviceMeterBus.peaks.get(name);
      const peak = watched ?? (active ? meterBus.latest.raw_peak_dbfs : SILENCE_DB);
      level.update(peak, now);
      const lit = Math.round(Math.min(Math.max((level.level + 60) / 60, 0), 1) * segments);
      const context = surface.begin(lit);
      if (!context) return;
      const { width, height } = surface;
      const step = width / segments;
      for (let i = 0; i < segments; i++) {
        context.fillStyle = i < lit ? colors[0] : colors[1];
        context.fillRect(i * step, 0, Math.max(1, step - 1), height);
      }
    });
  });
</script>

<canvas bind:this={canvas} class="level" style:--segments={segments} aria-hidden="true"></canvas>

<style>
  .level {
    width: calc(var(--segments) * 4px - 1px);
    height: 10px;
    flex: none;
  }
</style>
