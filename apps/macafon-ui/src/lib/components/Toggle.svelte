<script lang="ts">
  import { tip } from './tooltip.svelte';
  // On/off switch: square track, the knob carries the module colour when on.
  interface Props {
    checked: boolean;
    label: string;
    onchange: (checked: boolean) => void;
    color?: string;
    disabled?: boolean;
  }

  let { checked, label, onchange, color = 'blue', disabled = false }: Props = $props();
</script>

<button
  class="toggle"
  class:on={checked}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {@attach tip(label)}
  {disabled}
  style:--c="var(--app-{color})"
  onclick={() => onchange(!checked)}
>
  <span class="knob"></span>
</button>

<style>
  .toggle {
    position: relative;
    width: 30px;
    height: 16px;
    border: 1px solid var(--line-strong);
    background: var(--bg-sunken);
    flex: none;
    transition: border-color var(--dur) var(--ease);
  }

  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 10px;
    height: 10px;
    background: var(--faint);
    transition:
      transform var(--dur) var(--ease),
      background var(--dur) var(--ease);
  }

  .on {
    border-color: var(--c);
  }

  .on .knob {
    transform: translateX(14px);
    background: var(--c);
  }

  .toggle:disabled {
    opacity: 0.4;
  }
</style>
