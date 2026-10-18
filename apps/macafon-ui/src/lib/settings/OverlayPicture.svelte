<script lang="ts">
  // The quick controls overlay (HK-007) on a miniature monitor: centered in its upper middle,
  // with the soundpad bar under the sliders when it is on (SND-006).
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
    <span class="head"><span class="title"></span><span class="close"></span></span>
    <span class="body">
      <span class="sliders">
        {#each SLIDERS as slider (slider.color)}
          <span class="pill"><span class="level" style:height="{slider.level}%" style:background="var(--app-{slider.color})"></span></span>
        {/each}
      </span>
      <span class="tiles"><span></span><span></span><span></span></span>
    </span>
    {#if soundpad}
      <span class="sounds">
        <span class="chip"><span class="wave"></span></span>
        <span class="pads"><span></span><span></span><span></span><span></span></span>
      </span>
    {/if}
  </span>
</ScreenPicture>

<style>
  /* The overlay's rounded corners are an owner exception to the design system (HK-007). */
  .overlay {
    position: absolute;
    left: 24cqw;
    top: 5cqw;
    width: 52cqw;
    display: flex;
    flex-direction: column;
    gap: 1.4cqw;
    padding: 1.6cqw;
    border: 1px solid var(--line-strong);
    border-radius: 2cqw;
    background: var(--bg-raised);
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

  .sounds {
    display: flex;
    align-items: center;
    gap: 1.4cqw;
    padding-top: 1.2cqw;
    border-top: 1px solid var(--line);
  }

  .chip {
    display: flex;
    align-items: center;
    width: 9cqw;
    height: 3cqw;
    padding-left: 1cqw;
    border: 1px solid var(--accent-ink);
    border-radius: 0.8cqw;
  }

  .wave {
    width: 2.4cqw;
    height: 1.2cqw;
    background: repeating-linear-gradient(90deg, var(--accent-ink) 0 1px, transparent 1px 0.6cqw);
  }

  .pads {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0.8cqw;
  }

  .pads span {
    height: 3cqw;
    border: 1px solid var(--line);
    border-radius: 0.8cqw;
    background: var(--bg-sunken);
  }
</style>
