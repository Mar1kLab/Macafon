<script lang="ts">
  // The two looks of the hotkey popup and the overlay (HK-006, HK-007), close up: a volume
  // popup and a toggle side by side. Classic is the rounded pill; Sketch is the app's own
  // style with sharp corners, a ruler track and corner ticks.
  import type { OsdLook } from '../ipc/protocol';

  let { look }: { look: OsdLook } = $props();
</script>

<div class="look {look}" aria-hidden="true">
  <span class="popup">
    {#if look === 'sketch'}<span class="corners"><i></i><i></i><i></i><i></i></span>{/if}
    <span class="name"></span>
    <span class="track">
      <span class="fill"></span>
      {#if look === 'sketch'}
        {#each [10, 20, 30, 40, 50, 60, 70, 80, 90] as tick (tick)}
          <span class="tick" class:major={tick === 50} style:bottom="{tick}%"></span>
        {/each}
        <span class="head"></span>
      {/if}
    </span>
    <span class="icon"></span>
  </span>
  <span class="tiles">
    <span class="tile"><span class="dot"></span><span class="bar"></span></span>
    <span class="tile"><span class="dot"></span><span class="bar"></span></span>
    <span class="tile"><span class="dot"></span><span class="bar"></span></span>
  </span>
</div>

<style>
  .look {
    container-type: inline-size;
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 9;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8cqw;
    overflow: hidden;
    border: 1px solid var(--line);
    background-color: var(--bg-sunken);
    background-image:
      linear-gradient(var(--scanline) 1px, transparent 1px),
      linear-gradient(90deg, var(--scanline) 1px, transparent 1px);
    background-size: 8% 14.2%;
  }

  .popup {
    position: relative;
    width: 13cqw;
    height: 44cqw;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2cqw;
    padding: 3cqw 0;
    border: 1px solid var(--line-strong);
    background: var(--bg-raised);
  }

  .name {
    width: 6cqw;
    height: 1.2cqw;
    background: var(--muted);
  }

  .track {
    position: relative;
    flex: 1;
    width: 5cqw;
    overflow: hidden;
    background: var(--bg-sunken);
  }

  .fill {
    position: absolute;
    inset: auto 0 0;
    height: 62%;
    background: var(--app-blue);
  }

  .icon {
    width: 3cqw;
    height: 3cqw;
    border: 1px solid var(--app-blue-ink);
  }

  .tiles {
    display: flex;
    flex-direction: column;
    gap: 2cqw;
  }

  .tile {
    width: 30cqw;
    height: 8cqw;
    display: flex;
    align-items: center;
    gap: 2cqw;
    padding: 0 2.4cqw;
    border: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .dot {
    width: 2.6cqw;
    height: 2.6cqw;
    border: 1px solid var(--accent-ink);
  }

  .bar {
    width: 12cqw;
    height: 1.2cqw;
    background: var(--muted);
  }

  /* Classic: the owner-approved rounded pill and tiles (HK-006). */
  .classic .popup {
    border-radius: 6.5cqw;
    box-shadow: 0 1cqw 3cqw rgb(0 0 0 / 0.45);
    border-color: var(--line);
  }

  .classic .track {
    border-radius: 2.5cqw;
  }

  .classic .icon,
  .classic .dot {
    border-radius: 50%;
  }

  .classic .tile {
    border-radius: 2cqw;
  }

  .classic .name,
  .classic .bar {
    border-radius: 1cqw;
  }

  /* Sketch: the app's own look. */
  .sketch .track {
    border: 1px solid var(--line);
  }

  .sketch .name {
    height: 1cqw;
    background: var(--faint);
  }

  .tick {
    position: absolute;
    left: 0;
    width: 35%;
    height: 1px;
    background: var(--faint);
  }

  .tick.major {
    width: 60%;
    background: var(--line-strong);
  }

  .head {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 62%;
    height: 2px;
    background: var(--ink-strong);
  }

  .corners i {
    position: absolute;
    width: 3cqw;
    height: 3cqw;
    border: 0 solid var(--line-strong);
  }

  .corners i:nth-child(1) {
    left: -1.2cqw;
    top: -1.2cqw;
    border-width: 1px 0 0 1px;
  }

  .corners i:nth-child(2) {
    right: -1.2cqw;
    top: -1.2cqw;
    border-width: 1px 1px 0 0;
  }

  .corners i:nth-child(3) {
    left: -1.2cqw;
    bottom: -1.2cqw;
    border-width: 0 0 1px 1px;
  }

  .corners i:nth-child(4) {
    right: -1.2cqw;
    bottom: -1.2cqw;
    border-width: 0 1px 1px 0;
  }
</style>
