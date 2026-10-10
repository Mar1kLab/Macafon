<script lang="ts">
  // 11×11 crosshair ticks in the corners of an active area (design system §4).
  let { color = 'var(--line-strong)' }: { color?: string } = $props();
</script>

<span class="tick tl" style:--tick={color} aria-hidden="true"></span>
<span class="tick tr" style:--tick={color} aria-hidden="true"></span>
<span class="tick bl" style:--tick={color} aria-hidden="true"></span>
<span class="tick br" style:--tick={color} aria-hidden="true"></span>

<style>
  .tick {
    position: absolute;
    width: 11px;
    height: 11px;
    pointer-events: none;
    background:
      linear-gradient(var(--tick), var(--tick)) center / 1px 100% no-repeat,
      linear-gradient(var(--tick), var(--tick)) center / 100% 1px no-repeat;
  }

  .tl {
    top: -6px;
    left: -6px;
  }

  .tr {
    top: -6px;
    right: -6px;
  }

  .bl {
    bottom: -6px;
    left: -6px;
  }

  .br {
    bottom: -6px;
    right: -6px;
  }
</style>
