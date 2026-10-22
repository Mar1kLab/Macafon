<script lang="ts">
  // Two miniature monitors, the focused window on the second: the popup and the overlay follow
  // that window or stay on the primary monitor (HK-006, HK-007).
  import ScreenPicture from './ScreenPicture.svelte';

  let { primary }: { primary: boolean } = $props();
</script>

<div class="monitors">
  {#each [1, 2] as monitor (monitor)}
    <ScreenPicture label={String(monitor)} focused={monitor === 2}>
      {#if (monitor === 1) === primary}<span class="popup" aria-hidden="true"></span>{/if}
    </ScreenPicture>
  {/each}
</div>

<style>
  .monitors {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }

  /* The popup's rounded corners are an owner exception to the design system (HK-006). */
  .popup {
    position: absolute;
    right: 3cqw;
    top: 15cqw;
    width: 7cqw;
    height: 24cqw;
    overflow: hidden;
    border: 1px solid var(--line-strong);
    border-radius: 3.5cqw;
    background: linear-gradient(to top, var(--app-blue) 60%, var(--bg-raised) 60%);
  }
</style>
