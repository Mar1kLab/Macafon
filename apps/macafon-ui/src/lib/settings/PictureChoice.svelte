<script lang="ts" generics="T">
  // A radio group of picture cards: what an option looks like, its name and one line on it.
  import type { Snippet } from 'svelte';
  import Ticks from '../components/Ticks.svelte';

  interface Props {
    label: string;
    options: { value: T; label: string; hint?: string }[];
    value: T;
    picture: Snippet<[T]>;
    onchange: (value: T) => void;
  }

  let { label, options, value, picture, onchange }: Props = $props();
  let group: HTMLDivElement | undefined = $state();

  function onkeydown(event: KeyboardEvent) {
    const delta = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!delta) return;
    event.preventDefault();
    const index = options.findIndex((option) => option.value === value);
    const next = (index + delta + options.length) % options.length;
    onchange(options[next]!.value);
    group?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  }
</script>

<div class="choices" role="radiogroup" aria-label={label} tabindex="-1" bind:this={group} {onkeydown}>
  {#each options as option (option.label)}
    {@const selected = option.value === value}
    <button role="radio" aria-checked={selected} tabindex={selected ? 0 : -1} class:selected onclick={() => onchange(option.value)}>
      {#if selected}<Ticks color="var(--accent-ink)" />{/if}
      {@render picture(option.value)}
      <span class="text">
        <span class="name">{option.label}</span>
        {#if option.hint}<span class="hint">{option.hint}</span>{/if}
      </span>
    </button>
  {/each}
</div>

<style>
  .choices {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: var(--space-3);
  }

  button {
    position: relative;
    display: grid;
    gap: var(--space-2);
    align-content: start;
    padding: 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--muted);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: border-color var(--dur) var(--ease);
  }

  button:hover {
    border-color: var(--line-strong);
  }

  button:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 2px;
  }

  button.selected {
    border-color: var(--accent);
    color: var(--ink);
  }

  .text {
    display: grid;
    gap: var(--space-half);
  }

  .name {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 500;
    letter-spacing: 0.15em;
    text-transform: uppercase;
  }

  .selected .name {
    color: var(--accent-ink);
  }

  .hint {
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--muted);
  }
</style>
