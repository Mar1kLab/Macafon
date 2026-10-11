<script lang="ts">
  // Vertical stereo peak meter of a channel or the Master (CH-005, CH-006): 2px segments with
  // 1px gaps, colours of design system §2.2, held peak tick and a clip latch.
  import { onMount } from 'svelte';
  import { cssVar, fitCanvas, onFrame } from '../state/frame';
  import { Ballistics, SILENCE_DB, mixerMeterBus } from '../state/meters';

  let { id, color = 'blue' }: { id: string; color?: string } = $props();

  const MIN_DB = -60;
  let canvas: HTMLCanvasElement | undefined = $state();

  onMount(() => {
    const sides = [new Ballistics(), new Ballistics()];
    let colors: Record<string, string> | null = null;
    return onFrame((now) => {
      if (!canvas) return;
      colors ??= {
        ink: cssVar(canvas, `--app-${color}-ink`),
        amber: cssVar(canvas, '--app-amber-ink'),
        danger: cssVar(canvas, '--danger'),
        empty: cssVar(canvas, '--line'),
      };
      const peaks = mixerMeterBus.peaks.get(id) ?? [SILENCE_DB, SILENCE_DB];
      sides.forEach((side, index) => side.update(peaks[index]!, now));
      const context = fitCanvas(canvas);
      if (!context) return;
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      context.clearRect(0, 0, width, height);
      const bar = Math.floor((width - 2) / 2);
      const y = (db: number) => height - Math.min(Math.max((db - MIN_DB) / -MIN_DB, 0), 1) * height;
      const colorAt = (db: number) => (db > -1 ? colors!.danger! : db > -6 ? colors!.amber! : colors!.ink!);
      sides.forEach((side, index) => {
        const left = index * (bar + 2);
        const top = y(side.level);
        for (let bottom = height; bottom > 0; bottom -= 3) {
          const db = MIN_DB + ((height - bottom) / height) * -MIN_DB;
          const lit = bottom - 2 >= top;
          context.globalAlpha = lit && db < -18 ? 0.6 : 1;
          context.fillStyle = lit ? colorAt(db) : colors!.empty!;
          context.fillRect(left, bottom - 2, bar, 2);
        }
        context.globalAlpha = 1;
        if (side.peak > MIN_DB) {
          context.fillStyle = side.clipped ? colors!.danger! : colorAt(side.peak);
          context.fillRect(left, Math.max(y(side.peak), 0), bar, 2);
        }
      });
    });
  });
</script>

<canvas bind:this={canvas} class="meter" aria-hidden="true"></canvas>

<style>
  .meter {
    display: block;
    width: 14px;
    height: 100%;
  }
</style>
