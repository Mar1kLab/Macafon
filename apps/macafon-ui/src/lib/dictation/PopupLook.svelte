<script lang="ts">
  // The two looks of the dictation popup (PRD §9.9.3), drawn as the daemon draws them: Flow, a
  // small pill with mirrored level bars (rounded — an owner exception), and Analog, a vintage VU
  // meter in the app's own style. Only the voice: no timer, no language. `source` picks what
  // drives them: the live tap level, a speech-like demo, or nothing (a still picture).
  import { prefersReducedMotion } from 'svelte/motion';
  import type { DictationPopupLook } from '../ipc/protocol';
  import { onFrame } from '../state/frame';
  import { SILENCE_DB, dictationLevelBus } from '../state/meters';
  import { Needle, VU_MIN, dbfsToVu, vuToTravel } from './needle';
  import { t } from '../i18n';

  interface Props {
    look: DictationPopupLook;
    source?: 'live' | 'demo' | 'still';
    transcribing?: boolean;
  }

  let { look, source = 'demo', transcribing = false }: Props = $props();

  const BARS = 15;
  const ARC_DEG = 50;
  // The Analog look as the daemon draws it: 128×60, a thin arc, no numbers.
  const PIVOT = { x: 64, y: 42 };
  const RADIUS = 32;
  const MARKS = [-20, -10, -5, 0, 3];

  let travel = $state(vuToTravel(-3));
  let bars = $state<number[]>(Array.from({ length: BARS }, (_, i) => still(i)));
  let peak = $state(false);
  /** The held peak: a faint ghost needle that falls slowly. */
  let ghost = $state(0);
  let glowLevel = $state(0);
  let shimmer = $state(0);
  let demoStarted = 0;

  function still(i: number): number {
    return 0.25 + 0.55 * Math.abs(Math.sin(i * 1.7)) * window(i);
  }

  /** Center bars a little taller so the shape reads as a voice, not a spectrum. */
  function window(i: number): number {
    const x = (i - (BARS - 1) / 2) / ((BARS - 1) / 2);
    return 0.55 + 0.45 * Math.cos((x * Math.PI) / 2);
  }

  /** Speech-like RMS in dBFS for the demo: syllables around −18 dBFS with pauses. */
  function demoLevel(t: number): number {
    const phrase = t % 5;
    if (phrase > 3.6) return -52;
    const syllable = Math.max(0, Math.sin(t * Math.PI * 4.1)) ** 0.7;
    return -34 + 20 * syllable * (0.65 + 0.35 * Math.sin(t * 1.3));
  }

  function angle(t: number): number {
    return ((-ARC_DEG + 2 * ARC_DEG * t) * Math.PI) / 180;
  }

  function point(t: number, radius: number): { x: number; y: number } {
    const a = angle(t);
    return { x: PIVOT.x + radius * Math.sin(a), y: PIVOT.y - radius * Math.cos(a) };
  }

  function arc(from: number, to: number, radius: number): string {
    const a = point(from, radius);
    const b = point(to, radius);
    return `M ${a.x.toFixed(2)} ${a.y.toFixed(2)} A ${radius} ${radius} 0 0 1 ${b.x.toFixed(2)} ${b.y.toFixed(2)}`;
  }

  const red = vuToTravel(0);
  const needleTip = $derived(point(travel, RADIUS + 4));

  // Re-runs when `source` changes: a picture that becomes the selected one starts moving.
  $effect(() => {
    if (source === 'still') {
      travel = vuToTravel(-3);
      bars = Array.from({ length: BARS }, (_, i) => still(i));
      return;
    }
    const needle = new Needle();
    const smooth = new Array<number>(BARS).fill(0);
    const seeds = Array.from({ length: BARS }, (_, i) => 1.3 + ((i * 7919) % 13) / 4);
    let last = 0;
    demoStarted = performance.now();
    return onFrame((now) => {
      const dt = last ? (now - last) / 1000 : 0;
      last = now;
      const t = (now - demoStarted) / 1000;
      let rms = SILENCE_DB;
      let peakDb = SILENCE_DB;
      if (!transcribing) {
        if (source === 'live') {
          if (now - dictationLevelBus.receivedAt < 250) {
            rms = dictationLevelBus.rms;
            peakDb = dictationLevelBus.peak;
          }
        } else {
          rms = demoLevel(t);
          peakDb = rms + 9;
        }
      }
      const target = vuToTravel(Math.max(VU_MIN, dbfsToVu(rms)));
      travel = prefersReducedMotion.current ? needle.settle(target, dt) : needle.step(target, dt);
      peak = peakDb > -7;
      ghost = Math.max(travel, ghost - dt * 0.35);
      glowLevel = Math.max(0, Math.min(1, travel));
      shimmer = t;
      // Bars: 0…1 from −50…−10 dBFS, each with its own wobble; 40 ms attack, 160 ms release.
      const level = Math.min(1, Math.max(0, (rms + 50) / 40));
      bars = smooth.map((value, i) => {
        const wobble = 0.55 + 0.45 * Math.abs(Math.sin(t * seeds[i]! + i));
        const goal = Math.max(0.06, level * wobble * window(i));
        const tau = goal > value ? 0.04 : 0.16;
        const next = value + (goal - value) * Math.min(1, dt / tau);
        smooth[i] = next;
        return next;
      });
    });
  });
</script>

{#if look === 'flow'}
  <svg class="flow" class:transcribing viewBox="0 0 116 32" role="img" aria-hidden="true">
    <rect class="pill" x="0.5" y="0.5" width="115" height="31" rx="15.5" />
    {#if transcribing}
      {#each [0, 1, 2] as dot (dot)}
        <circle class="dot" cx={44 + dot * 14} cy="16" r="2.5" style:animation-delay="{dot * 150}ms" />
      {/each}
    {:else}
      {#each bars as height, i (i)}
        {@const h = 3 + height * 18}
        <rect class="bar" x={58 - (BARS * 6 - 3) / 2 + i * 6} y={16 - h / 2} width="3" height={h} rx="1.5" />
      {/each}
    {/if}
  </svg>
{:else}
  {@const sway = 0.18 + 0.1 * Math.sin(shimmer * 2.2)}
  {@const shown = transcribing ? sway : travel}
  {@const tip = point(shown, RADIUS - 3)}
  {@const ghostTip = point(ghost, RADIUS - 3)}
  <svg class="analog" class:transcribing viewBox="-6 -6 140 72" role="img" aria-hidden="true">
    <defs>
      <clipPath id="vu-frame"><rect x="0" y="0" width="128" height="60" /></clipPath>
    </defs>
    <rect class="frame" x="0.5" y="0.5" width="127" height="59" />
    {#each [[0, 0], [128, 0], [0, 60], [128, 60]] as [x, y] (`${x}-${y}`)}
      <path class="tick" d="M {x! - 4} {y} H {x! + 4} M {x} {y! - 4} V {y! + 4}" />
    {/each}
    <g clip-path="url(#vu-frame)">
      {#each [[38, 0.35], [26, 0.6], [15, 1]] as [r, a] (r)}
        <ellipse class="glow" cx={PIVOT.x} cy={PIVOT.y} rx={r} ry={r! * 0.8} style:opacity={(transcribing ? 0.04 : 0.05 + 0.16 * glowLevel) * a!} />
      {/each}
    </g>
    <path class="scale" d={arc(0, 1, RADIUS)} />
    <path class="red" d={arc(red, 1, RADIUS)} />
    {#each MARKS as vu (vu)}
      {@const at = vuToTravel(vu)}
      {@const inner = point(at, RADIUS - 4)}
      {@const outer = point(at, RADIUS)}
      <line class="mark" class:hot={vu >= 0} x1={inner.x} y1={inner.y} x2={outer.x} y2={outer.y} />
    {/each}
    {#if transcribing}
      {#each Array.from({ length: 6 }, (_, i) => i) as i (i)}
        {@const at = ((shimmer * 0.9) % 1) - i * 0.035}
        {#if at >= 0}
          {@const p = point(at, RADIUS)}
          <circle class="runner" cx={p.x} cy={p.y} r="1.6" style:opacity={1 - i / 6} />
        {/if}
      {/each}
    {/if}
    <text class="vu" x={PIVOT.x - 20} y={PIVOT.y - 4} text-anchor="middle">VU</text>
    {#if !transcribing && ghost > shown + 0.02}
      <line class="ghost" x1={PIVOT.x} y1={PIVOT.y} x2={ghostTip.x} y2={ghostTip.y} />
    {/if}
    <line class="needle" x1={PIVOT.x} y1={PIVOT.y} x2={tip.x} y2={tip.y} />
    <circle class="pivot" cx={PIVOT.x} cy={PIVOT.y} r="2.5" />
    {#if peak && !transcribing}<rect class="lamp on" x="116" y="8" width="4" height="4" />{/if}
    {#if transcribing}
      <text class="hud" x="8" y="56">{t('dictation.popup.transcribing')}</text>
    {:else}
      <circle class="rec" cx="10.5" cy="52.5" r="2" />
    {/if}
  </svg>
{/if}

<style>
  svg {
    display: block;
    width: 100%;
    height: auto;
    overflow: visible;
  }

  .pill {
    fill: color-mix(in srgb, var(--invert-fg) 92%, transparent);
    stroke: var(--line);
  }

  .bar {
    fill: var(--ink-strong);
  }

  .dot {
    fill: var(--ink-strong);
    animation: shimmer var(--dur-flash) var(--ease) infinite;
  }


  .frame {
    fill: var(--bg-raised);
    stroke: var(--line-strong);
  }

  .tick {
    stroke: var(--line-strong);
  }





  .scale {
    fill: none;
    stroke: var(--ink);
    stroke-width: 1;
  }

  .red {
    fill: none;
    stroke: var(--danger);
    stroke-width: 3;
  }

  .mark {
    stroke: var(--ink);
    stroke-width: 1;
  }

  .mark.hot {
    stroke: var(--danger);
  }



  .vu {
    font-family: var(--font-serif);
    font-style: italic;
    font-size: var(--text-lg);
    fill: var(--muted);
  }

  .lamp {
    fill: var(--bg-raised);
    stroke: var(--line);
  }

  .lamp.on {
    fill: var(--danger);
    stroke: var(--danger);
  }

  .needle {
    stroke: var(--ink-strong);
    stroke-width: 1.5;
    stroke-linecap: square;
  }



  .hud {
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    font-weight: 500;
    letter-spacing: 0.12em;
    fill: var(--ink);
  }



  .rec {
    fill: var(--danger);
    animation: blink var(--dur-loop) steps(1) infinite;
  }


  @keyframes shimmer {
    0%,
    100% {
      opacity: 0.25;
    }
    50% {
      opacity: 1;
    }
  }

  @keyframes blink {
    50% {
      opacity: 0.2;
    }
  }


  @media (prefers-reduced-motion: reduce) {
    .dot,
    .rec {
      animation: none;
    }
  }

  .glow {
    fill: var(--app-amber);
  }

  .ghost {
    stroke: var(--ink);
    stroke-opacity: 0.22;
    stroke-width: 1;
  }

  .pivot {
    fill: var(--ink-strong);
  }

  .runner {
    fill: var(--app-amber-ink);
  }
</style>
