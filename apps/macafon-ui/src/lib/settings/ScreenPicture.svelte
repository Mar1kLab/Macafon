<script lang="ts">
  // A miniature monitor for the Settings pictures: the work area with a window being worked
  // in and the taskbar. What it shows (popup, overlay) comes as children, placed in `cqw`
  // units of this screen's width.
  import type { Snippet } from 'svelte';

  interface Props {
    children?: Snippet;
    /** A caption in the corner, the monitor number. */
    label?: string;
    /** The window has the focus: this is the active monitor. */
    focused?: boolean;
  }

  let { children, label, focused = false }: Props = $props();
</script>

<div class="screen">
  <span class="window" class:focused aria-hidden="true"><span class="bar"></span></span>
  <span class="taskbar" aria-hidden="true"></span>
  {#if label}<span class="label">{label}</span>{/if}
  {@render children?.()}
</div>

<style>
  .screen {
    position: relative;
    container-type: inline-size;
    width: 100%;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    border: 1px solid var(--line);
    background-color: var(--bg-sunken);
    background-image:
      linear-gradient(var(--scanline) 1px, transparent 1px),
      linear-gradient(90deg, var(--scanline) 1px, transparent 1px);
    background-size: 8% 14.2%;
  }

  .window {
    position: absolute;
    left: 22%;
    top: 14%;
    width: 56%;
    height: 62%;
    border: 1px solid var(--line);
    background: var(--bg);
  }

  .bar {
    display: block;
    height: 8%;
    border-bottom: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .window.focused {
    border-color: var(--accent-ink);
  }

  .window.focused .bar {
    border-bottom-color: var(--accent-ink);
  }

  .taskbar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 7%;
    border-top: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .label {
    position: absolute;
    left: 3%;
    top: 4%;
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--faint);
  }
</style>
