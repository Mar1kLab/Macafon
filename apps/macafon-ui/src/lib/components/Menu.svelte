<script lang="ts" module>
  export type MenuEntry =
    | { kind: 'heading'; label: string }
    | { kind: 'separator' }
    | { kind: 'item'; label: string; onselect: () => void; disabled?: boolean; kbd?: string; swatch?: string; danger?: boolean }
    | { kind: 'check'; label: string; checked: boolean; onselect: () => void; disabled?: boolean }
    | { kind: 'radio'; value: string; options: { value: string; label: string }[]; onselect: (value: string) => void };

  /** Where the menu opens: at the pointer (a context menu) or under an element (keyboard). */
  export type MenuAnchor = { x: number; y: number } | HTMLElement;
</script>

<script lang="ts">
  // The shared context menu and dropdown (UIP-003) on Bits UI: arrow keys, typeahead, Escape,
  // focus return and collision handling come from it; the look is Sketch — a sunken panel,
  // 1 px frame, corner ticks, HUD headings.
  import { DropdownMenu } from 'bits-ui';
  import Ticks from './Ticks.svelte';

  interface Props {
    /** Open at this anchor; null closes. */
    anchor: MenuAnchor | null;
    label: string;
    entries: MenuEntry[];
    onclose: () => void;
  }

  let { anchor, label, entries, onclose }: Props = $props();

  const point = $derived(
    anchor && !(anchor instanceof HTMLElement)
      ? { getBoundingClientRect: () => DOMRect.fromRect({ x: anchor.x, y: anchor.y, width: 0, height: 0 }) }
      : anchor,
  );
</script>

<DropdownMenu.Root open={anchor !== null} onOpenChange={(open) => !open && onclose()}>
  {#if point}
    <DropdownMenu.Portal>
      <DropdownMenu.Content
        class="sketch-menu"
        aria-label={label}
        customAnchor={point}
        side="bottom"
        align="start"
        sideOffset={anchor instanceof HTMLElement ? 4 : 0}
        collisionPadding={8}
        loop
      >
        <Ticks />
        {#each entries as entry, index (index)}
          {#if entry.kind === 'heading'}
            <div class="heading hud-label" role="presentation">{entry.label}</div>
          {:else if entry.kind === 'separator'}
            <DropdownMenu.Separator class="separator" />
          {:else if entry.kind === 'item'}
            <DropdownMenu.Item class={entry.danger ? 'item danger' : 'item'} disabled={entry.disabled} onSelect={entry.onselect}>
              {#if entry.swatch}<span class="swatch" style:background={entry.swatch}></span>{/if}
              <span class="text">{entry.label}</span>
              {#if entry.kbd}<kbd>{entry.kbd}</kbd>{/if}
            </DropdownMenu.Item>
          {:else if entry.kind === 'check'}
            <DropdownMenu.CheckboxItem class="item" checked={entry.checked} disabled={entry.disabled} onCheckedChange={entry.onselect}>
              {#snippet children({ checked })}
                <span class="mark" class:on={checked} aria-hidden="true"></span>
                <span class="text">{entry.label}</span>
              {/snippet}
            </DropdownMenu.CheckboxItem>
          {:else}
            {@const radio = entry}
            <DropdownMenu.RadioGroup value={radio.value} onValueChange={radio.onselect}>
              {#each radio.options as option (option.value)}
                <DropdownMenu.RadioItem class="item" value={option.value}>
                  {#snippet children({ checked })}
                    <span class="mark" class:on={checked} aria-hidden="true"></span>
                    <span class="text">{option.label}</span>
                  {/snippet}
                </DropdownMenu.RadioItem>
              {/each}
            </DropdownMenu.RadioGroup>
          {/if}
        {/each}
      </DropdownMenu.Content>
    </DropdownMenu.Portal>
  {/if}
</DropdownMenu.Root>

<style>
  :global(.sketch-menu) {
    position: relative;
    z-index: 900;
    min-width: 180px;
    max-width: 280px;
    padding: var(--space-1);
    border: 1px solid var(--line-strong);
    background: var(--bg-sunken);
    outline: none;
    animation: menu-in var(--dur-reveal) var(--ease-out);
  }

  .heading {
    padding: var(--space-1) var(--space-2) var(--space-2);
    font-size: var(--text-2xs);
  }

  :global(.sketch-menu .separator) {
    height: 1px;
    margin: var(--space-1) 0;
    background: var(--line);
  }

  :global(.sketch-menu .item) {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: 28px;
    padding: 0 var(--space-2);
    border-left: 2px solid transparent;
    font-size: var(--text-md);
    color: var(--ink);
    cursor: pointer;
    outline: none;
    user-select: none;
  }

  /* Accent marks instead of fills (UIP-005): the highlighted item gets a left rule. */
  :global(.sketch-menu .item[data-highlighted]) {
    border-left-color: var(--accent);
    background: var(--bg-raised);
    color: var(--ink-strong);
  }

  :global(.sketch-menu .item[data-disabled]) {
    color: var(--faint);
    cursor: default;
  }

  :global(.sketch-menu .item.danger) {
    color: var(--danger);
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .swatch {
    flex: none;
    width: 8px;
    height: 8px;
  }

  .mark {
    flex: none;
    width: 8px;
    height: 8px;
    border: 1px solid var(--line-strong);
  }

  .mark.on {
    border-color: var(--accent-ink);
    background: var(--accent-ink);
  }

  kbd {
    padding: 0 var(--space-1);
    border: 1px solid var(--line);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    color: var(--muted);
  }

  @keyframes menu-in {
    from {
      opacity: 0;
      transform: translateY(-2px);
    }
  }
</style>
