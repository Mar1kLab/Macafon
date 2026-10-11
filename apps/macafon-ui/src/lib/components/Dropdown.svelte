<script lang="ts" generics="T">
  // Listbox dropdown with keyboard support (arrows, Enter, Escape, type-ahead free).
  import { tick, type Snippet } from 'svelte';
  import { ChevronDown } from '@lucide/svelte';

  interface Option {
    value: T;
    label: string;
    group?: string;
    disabled?: boolean;
  }

  interface Props {
    options: Option[];
    value: T;
    label: string;
    onchange: (value: T) => void;
    /** Content of the closed button; defaults to the selected label. */
    button?: Snippet;
    option?: Snippet<[Option, boolean]>;
    footer?: Snippet<[() => void]>;
    onopen?: () => void;
    onclose?: () => void;
    width?: string;
    disabled?: boolean;
  }

  let { options, value, label, onchange, button, option, footer, onopen, onclose, width, disabled = false }: Props =
    $props();

  let open = $state(false);
  let active = $state(0);
  let root: HTMLDivElement | undefined = $state();
  let list: HTMLUListElement | undefined = $state();
  const id = `dropdown-${Math.random().toString(36).slice(2)}`;

  const selected = $derived(options.find((o) => o.value === value));

  function show() {
    if (disabled) return;
    open = true;
    active = Math.max(0, options.findIndex((o) => o.value === value));
    onopen?.();
    void tick().then(() => list?.focus());
  }

  function close() {
    if (!open) return;
    open = false;
    onclose?.();
  }

  function choose(index: number) {
    const picked = options[index];
    if (!picked || picked.disabled) return;
    onchange(picked.value);
    close();
  }

  function move(delta: number) {
    if (options.length === 0) return;
    let next = active;
    for (let i = 0; i < options.length; i++) {
      next = (next + delta + options.length) % options.length;
      if (!options[next]?.disabled) break;
    }
    active = next;
  }

  function onkeydown(event: KeyboardEvent) {
    const keys: Record<string, () => void> = {
      ArrowDown: () => move(1),
      ArrowUp: () => move(-1),
      Enter: () => choose(active),
      ' ': () => choose(active),
      Escape: close,
      Tab: close,
    };
    const action = keys[event.key];
    if (action) {
      if (event.key !== 'Tab') event.preventDefault();
      action();
    }
  }

  $effect(() => {
    if (!open) return;
    const onpointer = (event: PointerEvent) => {
      if (root && !root.contains(event.target as Node)) close();
    };
    window.addEventListener('pointerdown', onpointer, true);
    return () => window.removeEventListener('pointerdown', onpointer, true);
  });
</script>

<div class="dropdown" bind:this={root} style:width>
  <button
    class="trigger"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={id}
    aria-label={label}
    {disabled}
    onclick={() => (open ? close() : show())}
    onkeydown={(event) => {
      if (event.key === 'ArrowDown' && !open) {
        event.preventDefault();
        show();
      }
    }}
  >
    <span class="current">
      {#if button}{@render button()}{:else}{selected?.label ?? ''}{/if}
    </span>
    <ChevronDown size={14} strokeWidth={1.5} />
  </button>
  {#if open}
    <div class="panel">
      <ul
        {id}
        bind:this={list}
        role="listbox"
        tabindex="-1"
        aria-label={label}
        aria-activedescendant="{id}-{active}"
        {onkeydown}
      >
        {#each options as item, index (index)}
          {#if item.group && item.group !== options[index - 1]?.group}
            <li class="group hud-label" role="presentation">{item.group}</li>
          {/if}
          <li
            id="{id}-{index}"
            role="option"
            aria-selected={item.value === value}
            aria-disabled={item.disabled}
            class:active={index === active}
            class:selected={item.value === value}
            class:disabled={item.disabled}
            onpointerenter={() => (active = index)}
            onclick={() => choose(index)}
            onkeydown={() => {}}
          >
            {#if option}{@render option(item, item.value === value)}{:else}{item.label}{/if}
          </li>
        {/each}
      </ul>
      {#if footer}
        <div class="footer">{@render footer(close)}</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .dropdown {
    position: relative;
    min-width: 0;
  }

  .trigger {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 30px;
    padding: 0 8px 0 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    text-align: left;
    color: var(--ink);
    transition: border-color var(--dur) var(--ease);
  }

  .trigger:hover:not(:disabled),
  .trigger[aria-expanded='true'] {
    border-color: var(--line-strong);
  }

  .trigger:disabled {
    color: var(--faint);
  }

  .current {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .panel {
    position: absolute;
    z-index: 30;
    top: calc(100% + 4px);
    left: 0;
    min-width: 100%;
    background: var(--bg-raised);
    border: 1px solid var(--line-strong);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    max-height: 320px;
    overflow-y: auto;
    outline: none;
  }

  li[role='option'] {
    padding: 6px 12px;
    cursor: pointer;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  li.active {
    background: var(--bg-sunken);
  }

  li.selected {
    color: var(--accent-ink);
  }

  li.disabled {
    color: var(--faint);
    cursor: default;
  }

  .group {
    padding: 8px 12px 2px;
    font-size: 10px;
    color: var(--faint);
  }

  .footer {
    border-top: 1px solid var(--line);
    padding: 6px;
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
