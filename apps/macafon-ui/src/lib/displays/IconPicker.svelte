<script lang="ts">
  // The icon of a display profile: a radio group of DISPLAY_PROFILE_ICONS.
  import { DISPLAY_PROFILE_ICONS } from '../ipc/protocol';
  import { t, type MessageKey } from '../i18n';
  import ProfileIcon from './ProfileIcon.svelte';

  let { value, onchange }: { value: string; onchange: (icon: string) => void } = $props();

  function onkeydown(event: KeyboardEvent) {
    const delta = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!delta) return;
    event.preventDefault();
    const index = DISPLAY_PROFILE_ICONS.indexOf(value as (typeof DISPLAY_PROFILE_ICONS)[number]);
    const next = DISPLAY_PROFILE_ICONS[(index + delta + DISPLAY_PROFILE_ICONS.length) % DISPLAY_PROFILE_ICONS.length]!;
    onchange(next);
    ((event.currentTarget as HTMLElement).querySelector(`[data-icon="${next}"]`) as HTMLElement | null)?.focus();
  }
</script>

<div class="icons" role="radiogroup" aria-label={t('displays.icon')} tabindex="-1" {onkeydown}>
  {#each DISPLAY_PROFILE_ICONS as icon (icon)}
    {@const label = t(`displays.icons.${icon}` as MessageKey)}
    <button
      type="button"
      class="icon"
      class:on={value === icon}
      role="radio"
      aria-checked={value === icon}
      aria-label={label}
      title={label}
      tabindex={value === icon ? 0 : -1}
      data-icon={icon}
      onclick={() => onchange(icon)}
    ><ProfileIcon {icon} size={16} /></button>
  {/each}
</div>

<style>
  .icons { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .icon { width: 34px; height: 34px; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--line); background: var(--bg-sunken); color: var(--muted); transition: border-color var(--dur) var(--ease), color var(--dur) var(--ease); }
  .icon:hover { border-color: var(--line-strong); color: var(--ink); }
  .icon.on { border-color: var(--accent); color: var(--accent-ink); }
  .icon:focus-visible { outline: 1px solid var(--ink); outline-offset: 2px; }
</style>
