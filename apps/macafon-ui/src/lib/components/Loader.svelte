<script lang="ts">
  // The shared loader (UIP-003): a 3×3 dot matrix that lights its dots in a ring, with a HUD
  // label. Reduced motion shows the matrix still.
  let { label, inline = false }: { label: string; inline?: boolean } = $props();

  // Ring order of the outer dots; the centre stays dim.
  const ORDER = [0, 1, 2, 5, 8, 7, 6, 3];
</script>

<div class="loader" class:inline role="status">
  <span class="matrix" aria-hidden="true">
    {#each Array.from({ length: 9 }, (_, index) => index) as index (index)}
      {@const step = ORDER.indexOf(index)}
      <i class:center={step < 0} style:--step={step}></i>
    {/each}
  </span>
  <span class="hud-label">{label}</span>
</div>

<style>
  .loader {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    height: 100%;
  }

  .inline {
    flex-direction: row;
    height: auto;
    gap: var(--space-2);
  }

  .matrix {
    display: grid;
    grid-template-columns: repeat(3, 4px);
    gap: var(--space-half);
  }

  i {
    width: 4px;
    height: 4px;
    background: var(--line-strong);
    animation: dot var(--dur-loop) steps(1) infinite;
    animation-delay: calc(var(--step) * var(--dur-loop) / 8);
  }

  i.center {
    background: var(--line);
    animation: none;
  }

  @keyframes dot {
    0% {
      background: var(--accent-ink);
    }
    25% {
      background: var(--line-strong);
    }
  }
</style>
