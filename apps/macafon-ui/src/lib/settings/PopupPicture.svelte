<script lang="ts">
  // The hotkey popup (HK-006) drawn on a miniature monitor where the daemon would show it:
  // the Android-like pill at a side or the Windows 11-like bar at the top or bottom, moved
  // toward the center by the offset. Sizes are the daemon's, enlarged a little to read.
  import type { OsdEdge, OsdStyle } from '../ipc/protocol';
  import ScreenPicture from './ScreenPicture.svelte';

  let { style, edge, offsetPct = 0 }: { style: OsdStyle; edge: OsdEdge; offsetPct?: number } = $props();

  // In `cqw` of the miniature: its height is 56.25, the taskbar takes 3.94 of it.
  const WORK_H = 52.3;
  const MARGIN = 1.5;
  const PILL = { w: 5, h: 20 };
  const BAR = { w: 25, h: 4.4 };

  const place = $derived.by(() => {
    const vertical = edge === 'left' || edge === 'right';
    const { w, h } = style === 'vertical' ? PILL : BAR;
    // The offset is a share of the screen across that edge, like the daemon's.
    const inset = MARGIN + (offsetPct / 100) * (vertical ? 100 : WORK_H);
    const along = vertical ? (WORK_H - h) / 2 : (100 - w) / 2;
    return { vertical, w, h, inset, along };
  });
</script>

<ScreenPicture>
  {#if offsetPct > 0}
    <span
      class="guide"
      class:across={place.vertical}
      style:left={edge === 'left' ? '0' : edge === 'right' ? 'auto' : '50cqw'}
      style:right={edge === 'right' ? '0' : 'auto'}
      style:top={edge === 'top' ? '0' : place.vertical ? `${WORK_H / 2}cqw` : 'auto'}
      style:bottom={edge === 'bottom' ? `${56.25 - WORK_H}cqw` : 'auto'}
      style:width={place.vertical ? `${place.inset}cqw` : '0'}
      style:height={place.vertical ? '0' : `${place.inset}cqw`}
      aria-hidden="true"
    ></span>
  {/if}
  <span
    class="popup {style}"
    style:width="{place.w}cqw"
    style:height="{place.h}cqw"
    style:left={edge === 'left' ? `${place.inset}cqw` : edge === 'right' ? 'auto' : `${place.along}cqw`}
    style:right={edge === 'right' ? `${place.inset}cqw` : 'auto'}
    style:top={edge === 'top' ? `${place.inset}cqw` : place.vertical ? `${place.along}cqw` : 'auto'}
    style:bottom={edge === 'bottom' ? `${56.25 - WORK_H + place.inset}cqw` : 'auto'}
    aria-hidden="true"
  >
    {#if style === 'horizontal'}<span class="track"></span>{/if}
    <span class="fill"></span>
    <span class="icon"></span>
  </span>
</ScreenPicture>

<style>
  .popup {
    position: absolute;
    overflow: hidden;
    border: 1px solid var(--line-strong);
    background: var(--bg-raised);
    transition:
      left var(--dur-card) var(--ease),
      right var(--dur-card) var(--ease),
      top var(--dur-card) var(--ease),
      bottom var(--dur-card) var(--ease);
  }

  /* The popup's rounded corners are an owner exception to the design system (HK-006). */
  .popup.vertical {
    border-radius: 2.5cqw;
  }

  .popup.horizontal {
    border-radius: 0.8cqw;
  }

  .fill {
    position: absolute;
    background: var(--app-blue);
  }

  .vertical .fill {
    left: 0;
    right: 0;
    bottom: 0;
    height: 62%;
  }

  .horizontal .fill {
    left: 18%;
    top: 44%;
    width: 48%;
    height: 12%;
    border-radius: 0.3cqw;
  }

  .track {
    position: absolute;
    left: 18%;
    right: 8%;
    top: 44%;
    height: 12%;
    border-radius: 0.3cqw;
    background: var(--line);
  }

  .icon {
    position: absolute;
    width: 1.8cqw;
    height: 1.8cqw;
    border-radius: 50%;
    border: 1px solid var(--ink-strong);
  }

  .vertical .icon {
    left: calc(50% - 0.9cqw);
    bottom: 1.4cqw;
  }

  .horizontal .icon {
    left: 4%;
    top: calc(50% - 0.9cqw);
    border-color: var(--app-blue-ink);
  }

  .guide {
    position: absolute;
    border: 0 dashed var(--accent-ink);
    transition:
      width var(--dur-card) var(--ease),
      height var(--dur-card) var(--ease);
  }

  .guide.across {
    border-top-width: 1px;
  }

  .guide:not(.across) {
    border-left-width: 1px;
  }

  @media (prefers-reduced-motion: reduce) {
    .popup,
    .guide {
      transition: none;
    }
  }
</style>
