<script lang="ts">
  // The quick controls overlay (HK-007) on a miniature monitor: centered in its upper middle,
  // in its sketch frame. With the soundpad on (SND-006) a round button under the toggles opens
  // the pads below.
  import ScreenPicture from './ScreenPicture.svelte';

  let { soundpad }: { soundpad: boolean } = $props();
  const SLIDERS = [
    { color: 'teal', level: 100 },
    { color: 'blue', level: 30 },
    { color: 'violet', level: 50 },
    { color: 'pink', level: 70 },
  ];
</script>

<ScreenPicture>
  <span class="overlay" aria-hidden="true">
    <span class="frame"><i class="t"></i><i class="b"></i><i class="l"></i><i class="r"></i></span>
    <span class="head"><span class="title"></span><span class="close"></span></span>
    <span class="body">
      <span class="sliders">
        {#each SLIDERS as slider (slider.color)}
          <span class="pill"><span class="level" style:height="{slider.level}%" style:background="var(--app-{slider.color})"></span></span>
        {/each}
      </span>
      <span class="tiles">
        <span></span><span></span><span></span>
        {#if soundpad}<span class="round"><span class="wave"></span></span>{/if}
      </span>
    </span>
    {#if soundpad}
      <span class="pads"><span></span><span></span><span></span><span></span><span></span></span>
    {/if}
  </span>
</ScreenPicture>

<style>
  .overlay {
    position: absolute;
    left: 24cqw;
    top: 5cqw;
    width: 52cqw;
    display: flex;
    flex-direction: column;
    gap: 1.4cqw;
    padding: 1.6cqw;
    background: var(--bg-raised);
  }

  .frame {
    position: absolute;
    inset: 0.6cqw;
  }

  .frame i {
    position: absolute;
    background: var(--line-strong);
  }

  .frame .t,
  .frame .b {
    left: -0.6cqw;
    right: -0.6cqw;
    height: 1px;
  }

  .frame .l,
  .frame .r {
    top: -0.6cqw;
    bottom: -0.6cqw;
    width: 1px;
  }

  .frame .t {
    top: 0;
  }

  .frame .b {
    bottom: 0;
  }

  .frame .l {
    left: 0;
  }

  .frame .r {
    right: 0;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: 1cqw;
    border-bottom: 1px solid var(--line);
  }

  .title {
    width: 12cqw;
    height: 1.2cqw;
    background: var(--muted);
  }

  .close {
    width: 2.2cqw;
    height: 2.2cqw;
    border: 1px solid var(--line-strong);
    border-radius: 50%;
  }

  .body {
    display: flex;
    gap: 2cqw;
  }

  .sliders {
    flex: 1.6;
    display: flex;
    justify-content: space-around;
  }

  .pill {
    position: relative;
    width: 3.6cqw;
    height: 13cqw;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 1.8cqw;
    background: var(--bg-sunken);
  }

  .level {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
  }

  .tiles {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1cqw;
    padding-left: 2cqw;
    border-left: 1px solid var(--line);
  }

  .tiles span {
    height: 3cqw;
    border: 1px solid var(--line);
    border-radius: 0.8cqw;
    background: var(--bg-sunken);
  }

  .tiles .round {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 3cqw;
    height: 3cqw;
    border: 1px solid var(--accent-ink);
    border-radius: 50%;
    background: none;
    align-self: flex-start;
  }

  .tiles .wave {
    width: 1.4cqw;
    height: 1cqw;
    border: 0;
    border-radius: 0;
    background: repeating-linear-gradient(90deg, var(--accent-ink) 0 1px, transparent 1px 0.4cqw);
  }

  .pads {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 0.8cqw;
  }

  .pads span {
    height: 3cqw;
    border: 1px solid var(--line);
    border-radius: 0.8cqw;
    background: var(--bg-sunken);
  }
</style>
