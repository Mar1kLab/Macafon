<script lang="ts">
  // The hotkey overlay with a side panel open on a miniature monitor: where the panel opens
  // (right, left, down) and whether the overlay stays put or keeps itself centered. A dashed
  // line marks the middle of the screen.
  import ScreenPicture from './ScreenPicture.svelte';
  import type { OverlayExpand, OverlayPlacement } from '../ipc/protocol';

  let { expand, placement }: { expand: OverlayExpand; placement: OverlayPlacement } = $props();
  const MAIN = 36;
  const SIDE = 22;
  const down = $derived(expand === 'down');
  /** Left edge of the whole overlay, in percent of the screen width. */
  const left = $derived(down ? 50 - MAIN / 2 : placement === 'centered' ? 50 - (MAIN + SIDE) / 2 : expand === 'right' ? 50 - MAIN / 2 : 50 - MAIN / 2 - SIDE);
</script>

<ScreenPicture>
  <span class="center" aria-hidden="true"></span>
  <span class="overlay" class:down class:reverse={expand === 'left'} style:left="{left}cqw" aria-hidden="true">
    <span class="main" style:width="{MAIN}cqw">
      <span class="sliders"><i></i><i></i><i></i><i></i></span>
      <span class="tiles"><i></i><i></i><i class="flag"></i></span>
    </span>
    <span class="side" style:width={down ? `${MAIN}cqw` : `${SIDE}cqw`}><i></i><i></i><i></i><i></i><i></i><i></i></span>
  </span>
</ScreenPicture>

<style>
  .center {
    position: absolute;
    left: 50%;
    top: 0;
    bottom: 7%;
    border-left: 1px dashed var(--line-strong);
  }

  .overlay {
    position: absolute;
    top: 7cqw;
    display: flex;
    border: 1px solid var(--line-strong);
    background: var(--bg-raised);
    transition: left var(--dur) var(--ease);
  }

  .overlay.reverse {
    flex-direction: row-reverse;
  }

  .overlay.down {
    flex-direction: column;
  }

  .main {
    display: flex;
    gap: 1.4cqw;
    height: 17cqw;
    padding: 1.4cqw;
    box-sizing: border-box;
  }

  .sliders {
    flex: 1.5;
    display: flex;
    justify-content: space-around;
  }

  .sliders i {
    width: 2.4cqw;
    border: 1px solid var(--line);
    border-radius: 1.2cqw;
    background: linear-gradient(to top, var(--app-teal) 55%, transparent 55%);
  }

  .sliders i:nth-child(2) {
    background: linear-gradient(to top, var(--app-blue) 35%, transparent 35%);
  }

  .sliders i:nth-child(3) {
    background: linear-gradient(to top, var(--app-violet) 70%, transparent 70%);
  }

  .sliders i:nth-child(4) {
    background: linear-gradient(to top, var(--app-pink) 45%, transparent 45%);
  }

  .tiles {
    flex: 1;
    display: grid;
    gap: 0.8cqw;
    align-content: start;
    padding-left: 1.4cqw;
    border-left: 1px solid var(--line);
  }

  .tiles i {
    height: 3cqw;
    border: 1px solid var(--line);
    border-radius: 0.6cqw;
  }

  .tiles .flag {
    border-color: var(--accent);
    background: var(--accent);
  }

  .side {
    box-sizing: border-box;
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 0.8cqw;
    align-content: start;
    padding: 1.4cqw;
    border-left: 1px solid var(--line);
  }

  .reverse .side {
    border-left: 0;
    border-right: 1px solid var(--line);
  }

  .down .side {
    grid-template-columns: repeat(3, 1fr);
    border-left: 0;
    border-top: 1px solid var(--line);
  }

  .side i {
    height: 3cqw;
    border: 1px solid var(--line);
    border-radius: 0.6cqw;
    background: var(--bg-sunken);
  }

  @media (prefers-reduced-motion: reduce) {
    .overlay {
      transition: none;
    }
  }
</style>
