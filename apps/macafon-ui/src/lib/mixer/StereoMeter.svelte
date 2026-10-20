<script lang="ts">
  // Vertical stereo peak meter of a channel or the Master (CH-005, CH-006): 2px segments with
  // 1px gaps, colours of design system §2.2, held peak tick and a clip latch.
  import { onMount } from 'svelte';
  import { cssVar, drawEveryFrame } from '../state/frame';
  import { Ballistics, SILENCE_DB, mixerMeterBus } from '../state/meters';

  let { id, color = 'blue' }: { id: string; color?: string } = $props();

  const MIN_DB = -60;
  let canvas: HTMLCanvasElement | undefined = $state();

  onMount(() => {
    const sides = [new Ballistics(), new Ballistics()];
    const colors = {
      ink: cssVar(canvas!, `--app-${color}-ink`),
      amber: cssVar(canvas!, '--app-amber-ink'),
      danger: cssVar(canvas!, '--danger'),
      empty: cssVar(canvas!, '--line'),
    };
    const colorAt = (db: number) => (db > -1 ? colors.danger : db > -6 ? colors.amber : colors.ink);
    return drawEveryFrame(canvas!, (surface, now) => {
      const peaks = mixerMeterBus.peaks.get(id) ?? [SILENCE_DB, SILENCE_DB];
      sides.forEach((side, index) => side.update(peaks[index]!, now));
      const { width, height } = surface;
      const y = (db: number) => height - Math.min(Math.max((db - MIN_DB) / -MIN_DB, 0), 1) * height;
      // Segments light at whole pixels: a level that moved within one draws the same picture.
      const tops = sides.map((side) => Math.ceil(y(side.level)));
      const ticks = sides.map((side) => (side.peak > MIN_DB ? Math.round(Math.max(y(side.peak), 0)) : -1));
      const context = surface.begin(`${tops}|${ticks}|${sides.map((side) => side.clipped)}`);
      if (!context) return;
      const bar = Math.floor((width - 2) / 2);
      sides.forEach((side, index) => {
        const left = index * (bar + 2);
        const top = tops[index]!;
        for (let bottom = height; bottom > 0; bottom -= 3) {
          const db = MIN_DB + ((height - bottom) / height) * -MIN_DB;
          const lit = bottom - 2 >= top;
          context.globalAlpha = lit && db < -18 ? 0.6 : 1;
          context.fillStyle = lit ? colorAt(db) : colors.empty;
          context.fillRect(left, bottom - 2, bar, 2);
        }
        context.globalAlpha = 1;
        const tick = ticks[index]!;
        if (tick >= 0) {
          context.fillStyle = side.clipped ? colors.danger : colorAt(side.peak);
          context.fillRect(left, tick, bar, 2);
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
