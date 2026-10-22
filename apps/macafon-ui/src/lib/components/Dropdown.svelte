<script lang="ts" module>
  const LIST_MAX_PX = 320;
  const MIN_LIST_PX = 96;
  const GAP_PX = 4;
</script>

<script lang="ts" generics="T">
  // Listbox dropdown with keyboard support (arrows, Enter, Escape, type-ahead free). The list
  // opens where it fits inside the window and every scrolling or clipping container around it:
  // below, or above when there is more room there, never growing the page under it.
  import { tick, type Snippet } from 'svelte';
  import { ChevronDown } from '@lucide/svelte';
  import { uiZoom } from './zoom';
  import { tip } from './tooltip.svelte';

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
  let panel: HTMLDivElement | undefined = $state();
  let upward = $state(false);
  let alignRight = $state(false);
  let trigger: HTMLButtonElement | undefined = $state();
  let maxHeight = $state(LIST_MAX_PX);
  const id = `dropdown-${Math.random().toString(36).slice(2)}`;

  /** The visible area the list may use: the window cut by every clipping ancestor. */
  function bounds(element: HTMLElement) {
    let top = 0;
    let bottom = window.innerHeight;
    for (let node = element.parentElement; node && node !== document.body; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.overflowY === 'visible' && style.overflowX === 'visible') continue;
      const rect = node.getBoundingClientRect();
      top = Math.max(top, rect.top);
      bottom = Math.min(bottom, rect.bottom);
    }
    return { top, bottom };
  }

  /** Below if the list fits there, else on the side with more room, scrolling inside. */
  function place() {
    if (!root || !list || !panel) return;
    // Rectangles are in screen pixels, sizes and styles in CSS pixels (the UI zoom).
    const zoom = uiZoom();
    const trigger = root.getBoundingClientRect();
    const area = bounds(root);
    const chrome = panel.offsetHeight - list.offsetHeight;
    const wanted = Math.min(list.scrollHeight, LIST_MAX_PX) + chrome;
    const below = (area.bottom - trigger.bottom) / zoom - GAP_PX * 2;
    const above = (trigger.top - area.top) / zoom - GAP_PX * 2;
    upward = below < wanted && above > below;
    alignRight = panel.getBoundingClientRect().right > window.innerWidth - GAP_PX * zoom;
    maxHeight = Math.max(MIN_LIST_PX, Math.min(LIST_MAX_PX, (upward ? above : below) - chrome));
  }

  const selected = $derived(options.find((o) => o.value === value));

  function show() {
    if (disabled) return;
    open = true;
    active = Math.max(0, options.findIndex((o) => o.value === value));
    upward = false;
    alignRight = false;
    maxHeight = LIST_MAX_PX;
    onopen?.();
    void tick().then(() => {
      place();
      list?.focus({ preventScroll: true });
    });
  }

  /** Keyboard closes give the focus back to the button, so it is not lost on the page. */
  function close(refocus = true) {
    if (!open) return;
    open = false;
    onclose?.();
    if (refocus) trigger?.focus();
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
    void tick().then(() => list?.querySelector(`#${id}-${active}`)?.scrollIntoView({ block: 'nearest' }));
  }

  function onkeydown(event: KeyboardEvent) {
    const keys: Record<string, () => void> = {
      ArrowDown: () => move(1),
      ArrowUp: () => move(-1),
      Enter: () => choose(active),
      ' ': () => choose(active),
      Escape: () => close(),
      Tab: () => close(),
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
      if (root && !root.contains(event.target as Node)) close(false);
    };
    window.addEventListener('pointerdown', onpointer, true);
    const onresize = () => close(false);
    window.addEventListener('resize', onresize);
    return () => {
      window.removeEventListener('pointerdown', onpointer, true);
      window.removeEventListener('resize', onresize);
    };
  });
</script>

<div class="dropdown" bind:this={root} style:width>
  <button
    bind:this={trigger}
    class="trigger"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={id}
    aria-label={label}
    {disabled}
    onclick={() => (open ? close(false) : show())}
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
    <div class="panel" class:upward class:right={alignRight} bind:this={panel}>
      <ul
        style:max-height="{maxHeight}px"
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
            {#if option}{@render option(item, item.value === value)}{:else}<span class="label" {@attach tip(item.label)}>{item.label}</span>{/if}
          </li>
        {/each}
      </ul>
      {#if footer}
        <div class="footer">{@render footer(() => close(false))}</div>
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
    gap: var(--space-2);
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
    gap: var(--space-2);
  }

  .panel {
    position: absolute;
    z-index: 30;
    top: calc(100% + 4px);
    left: 0;
    min-width: 100%;
    max-width: min(480px, calc(100vw / var(--zoom, 1) - 16px));
    background: var(--bg-raised);
    border: 1px solid var(--line-strong);
  }

  .panel.right {
    left: auto;
    right: 0;
  }

  .panel.upward {
    top: auto;
    bottom: calc(100% + 4px);
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
    gap: var(--space-2);
  }

  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
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
    font-size: var(--text-2xs);
    color: var(--faint);
  }

  .footer {
    border-top: 1px solid var(--line);
    padding: 6px;
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
