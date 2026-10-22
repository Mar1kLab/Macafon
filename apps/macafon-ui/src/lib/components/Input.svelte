<script lang="ts">
  // The one text field (UIP-003): sunken, 1 px frame, accent frame on focus. An optional leading
  // icon sits inside the frame (search fields). Every other attribute goes to the <input>.
  import type { Snippet } from 'svelte';
  import type { HTMLInputAttributes } from 'svelte/elements';

  type Props = Omit<HTMLInputAttributes, 'value' | 'size'> & {
    value?: string;
    /** 30 px, the height of the buttons next to it; the default is 32 px for forms. */
    compact?: boolean;
    icon?: Snippet;
    element?: HTMLInputElement;
  };

  let { value = $bindable(''), compact = false, icon, element = $bindable(), class: className = '', ...rest }: Props = $props();
</script>

<span class="ui-input {className}" class:compact class:disabled={rest.disabled}>
  {#if icon}<span class="icon" aria-hidden="true">{@render icon()}</span>{/if}
  <input bind:this={element} bind:value autocomplete="off" spellcheck="false" {...rest} />
</span>

<style>
  .ui-input {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--muted);
    transition: border-color var(--dur) var(--ease);
  }

  .ui-input.compact {
    height: 30px;
    padding: 0 var(--space-2);
  }

  .ui-input:hover:not(.disabled) {
    border-color: var(--line-strong);
  }

  .ui-input:focus-within {
    border-color: var(--accent);
  }

  .ui-input.disabled {
    color: var(--faint);
  }

  .icon {
    display: inline-flex;
    flex: none;
  }

  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: none;
    color: var(--ink);
    font-size: var(--text-md);
    outline: none;
    user-select: text;
  }

  input:disabled {
    color: var(--faint);
  }

  input::placeholder {
    color: var(--faint);
  }

  input[type='search']::-webkit-search-cancel-button {
    filter: grayscale(1);
  }
</style>
