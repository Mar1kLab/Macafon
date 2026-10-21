<script lang="ts" module>
  import type { DisplayLayout } from '../ipc/protocol';

  export interface SketchMonitor {
    key: string;
    name: string;
    layout: DisplayLayout | null;
    /** A profile monitor that is not connected now (DISP-007). */
    missing?: boolean;
  }

  /** The desktop size of a layout, turned with the monitor. */
  export function extent(layout: DisplayLayout): { width: number; height: number } {
    const turned = layout.rotation === 'rotate90' || layout.rotation === 'rotate270';
    return turned
      ? { width: layout.mode.height, height: layout.mode.width }
      : { width: layout.mode.width, height: layout.mode.height };
  }
</script>

<script lang="ts">
  // A to-scale sketch of a monitor layout (§9.10): the monitors that are on as frames in their
  // desktop positions, the main one marked; the ones that are off listed under it, dashed.
  import { t } from '../i18n';

  let { monitors, height = 96 }: { monitors: SketchMonitor[]; height?: number } = $props();

  const on = $derived(monitors.filter((monitor) => monitor.layout !== null));
  const off = $derived(monitors.filter((monitor) => monitor.layout === null));
  const bounds = $derived.by(() => {
    if (on.length === 0) return null;
    let left = Infinity;
    let top = Infinity;
    let right = -Infinity;
    let bottom = -Infinity;
    for (const monitor of on) {
      const { width, height } = extent(monitor.layout!);
      left = Math.min(left, monitor.layout!.x);
      top = Math.min(top, monitor.layout!.y);
      right = Math.max(right, monitor.layout!.x + width);
      bottom = Math.max(bottom, monitor.layout!.y + height);
    }
    return { left, top, width: right - left, height: bottom - top };
  });
  /** Mirrors share a frame: one per desktop position. */
  const frames = $derived.by(() => {
    const result: { monitors: SketchMonitor[]; layout: DisplayLayout }[] = [];
    for (const monitor of on) {
      const layout = monitor.layout!;
      const same = result.find((frame) => frame.layout.x === layout.x && frame.layout.y === layout.y && layout.mirror_of !== undefined);
      if (same) same.monitors.push(monitor);
      else result.push({ monitors: [monitor], layout });
    }
    return result;
  });

  const hz = (mhz: number) => `${Math.round(mhz / 1000)} Hz`;
  const label = (frame: { monitors: SketchMonitor[]; layout: DisplayLayout }) =>
    `${frame.monitors.map((monitor) => monitor.name).join(' + ')}: ${frame.layout.mode.width}×${frame.layout.mode.height}, ${hz(frame.layout.mode.refresh_mhz)}${frame.layout.primary ? `, ${t('displays.primary')}` : ''}`;
</script>

<div class="sketch" style:--h="{height}px" role="img" aria-label={[...frames.map(label), ...off.map((monitor) => `${monitor.name}: ${t('displays.off')}`)].join('; ')}>
  <div class="area">
    {#if bounds}
      <div class="desk" style:--ratio={bounds.width / bounds.height}>
        {#each frames as frame (frame.monitors[0]!.key)}
          {@const size = extent(frame.layout)}
          <span
            class="monitor"
            class:primary={frame.layout.primary}
            class:missing={frame.monitors.every((monitor) => monitor.missing)}
            style:left="{((frame.layout.x - bounds.left) / bounds.width) * 100}%"
            style:top="{((frame.layout.y - bounds.top) / bounds.height) * 100}%"
            style:width="{(size.width / bounds.width) * 100}%"
            style:height="{(size.height / bounds.height) * 100}%"
          >
            <span class="name">{frame.monitors.map((monitor) => monitor.name).join(' + ')}</span>
            <span class="mode">{frame.layout.mode.width}×{frame.layout.mode.height} · {hz(frame.layout.mode.refresh_mhz)}{frame.layout.hdr ? ' · HDR' : ''}</span>
          </span>
        {/each}
      </div>
    {/if}
  </div>
  {#if off.length}
    <div class="off">
      {#each off as monitor (monitor.key)}
        <span class="chip" class:missing={monitor.missing}>{monitor.name} · {t('displays.off')}</span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .sketch { display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .area { height: var(--h); display: flex; align-items: center; justify-content: center; min-width: 0; }
  /* As large as fits the area, keeping the desktop's proportions. */
  .desk { position: relative; width: min(100%, calc(var(--h) * var(--ratio))); aspect-ratio: var(--ratio); }
  .monitor {
    position: absolute;
    box-sizing: border-box;
    /* A small gap between neighbours, so touching monitors read as two. */
    border: 1px solid var(--line-strong);
    outline: 2px solid var(--bg-raised);
    outline-offset: -3px;
    background: var(--bg-sunken);
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    gap: 2px;
    padding: 2px 4px;
    overflow: hidden;
    min-width: 0;
  }
  .monitor.primary { border-color: var(--accent); }
  .monitor.primary::after { content: ''; position: absolute; left: 0; right: 0; top: 0; height: 2px; background: var(--accent); }
  .monitor.missing { opacity: 0.4; border-style: dashed; }
  .name { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11px; color: var(--ink); }
  .mode { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono); font-size: 9px; color: var(--muted); }
  .off { display: flex; flex-wrap: wrap; gap: 4px; }
  .chip { border: 1px dashed var(--line-strong); padding: 2px 6px; font-family: var(--font-mono); font-size: 10px; color: var(--muted); }
  .chip.missing { opacity: 0.5; }
</style>
