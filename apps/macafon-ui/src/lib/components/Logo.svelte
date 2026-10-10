<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { logoPaths } from '../brand/morph';
  import { cssVar, onFrame } from '../state/frame';

  let { inactive = false, bypass = false, error = false, label }: {
    inactive?: boolean;
    bypass?: boolean;
    error?: boolean;
    label: string;
  } = $props();

  let element: SVGSVGElement | undefined = $state();
  let amount = $state(0);
  let reduced = $state(true);
  const paths = $derived(logoPaths(amount));

  onMount(() => {
    const preference = matchMedia('(prefers-reduced-motion: reduce)');
    const sync = () => { reduced = preference.matches; };
    sync();
    preference.addEventListener('change', sync);
    return () => preference.removeEventListener('change', sync);
  });

  $effect(() => {
    const target = inactive ? 1 : 0;
    const from = untrack(() => amount);
    const duration = element ? parseFloat(cssVar(element, '--dur-card')) : 0;
    if (reduced || !Number.isFinite(duration) || duration <= 0 || target === from) {
      amount = target;
      return;
    }
    const started = performance.now();
    const stop = onFrame((now) => {
      const progress = Math.min(1, (now - started) / duration);
      const eased = 0.5 - 0.5 * Math.cos(Math.PI * progress);
      amount = from + (target - from) * eased;
      if (progress === 1) stop();
    });
    // Retarget from the visible shape after a rapid toggle; no stale callbacks.
    return stop;
  });
</script>

<svg bind:this={element} class:inactive data-state={error ? 'error' : bypass ? 'bypass' : inactive ? 'muted' : 'normal'}
  viewBox="-8 0 128 128" role="img" aria-label={label}>
  <title>{label}</title>
  <g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round">
    {#each paths as path, i (i)}
      <path d={path.d} stroke-width={path.width} />
    {/each}
    {#if bypass}<path d="M96 88V105M106 88V105" stroke-width="5" />{/if}
  </g>
  {#if error}<circle class="error" cx="105" cy="22" r="8" />{/if}
</svg>

<style>
  svg {
    display: block;
    width: 44px;
    height: 44px;
    color: var(--ink-strong);
    transition: color var(--dur-card) var(--ease);
  }
  svg.inactive { color: var(--danger); }
  .error { fill: var(--danger); }
</style>
