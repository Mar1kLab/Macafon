<script lang="ts">
  import { daemon } from '../state';
  import { t, type MessageKey } from '../i18n';
  import { displayShortcut } from './shortcut';
  let { actions }: { actions: string[] } = $props();
  const bindings = $derived((daemon.hotkeys?.bindings ?? []).filter((binding) => actions.includes(binding.action)));
</script>

<span class="hints">
  {#each bindings as binding (binding.action)}
    {@const label = t(`settings.hotkeys.actions.${binding.action.split('.')[0]}` as MessageKey)}
    {#each [binding.shortcut, binding.secondary_shortcut].filter(Boolean) as shortcut}
      <kbd title={label} aria-label={`${label}: ${displayShortcut(shortcut!)}`}>{displayShortcut(shortcut!)}</kbd>
    {/each}
  {/each}
</span>

<style>
  .hints { display: flex; flex-wrap: wrap; justify-content: center; gap: 4px; min-width: 0; }
  .hints:empty { display: none; }
  kbd { font-family: var(--font-mono); font-size: 9px; line-height: 1.5; color: var(--muted); overflow-wrap: anywhere; }
  kbd + kbd::before { content: '/ '; color: var(--faint); }
</style>
