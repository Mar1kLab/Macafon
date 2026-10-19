<script lang="ts">
  // The two looks of the dictation popup (PRD §9.9.3), drawn as the daemon draws them: Flow, a
  // Wispr Flow-like pill with mirrored level bars (rounded — an owner exception), and Analog, a
  // vintage VU meter in the app's own style. `source` picks what drives them: the live tap level,
  // a speech-like demo, or nothing (a still picture).
  import type { DictationPopupLook } from '../ipc/protocol';
  import { onFrame } from '../state/frame';
  import { SILENCE_DB, dictationLevelBus } from '../state/meters';
  import { Needle, VU_MIN, dbfsToVu, vuToTravel } from './needle';

  interface Props {
    look: DictationPopupLook;
    source?: 'live' | 'demo' | 'still';
    transcribing?: boolean;
    /** Shroud and pill details. */
    engine?: string;
    language?: string | null;
    elapsedMs?: number;
  }

  let { look, source = 'demo', transcribing = false, engine = 'WHISPER · TURBO', language = 'UK', elapsedMs }: Props = $props();

  const BARS = 15;
  const ARC_DEG = 50;
  const PIVOT = { x: 140, y: 120 };
  const RADIUS = 92;
  const SCALE: [number, string | null][] = [
    [-20, '20'], [-10, '10'], [-7, '7'], [-5, '5'], [-3, '3'], [-2, null], [-1, null], [0, '0'], [1, null], [2, null], [3, '3'],
  ];

  let travel = $state(vuToTravel(-3));
  let bars = $state<number[]>(Array.from({ length: BARS }, (_, i) => still(i)));
  let peak = $state(false);
  let clock = $state(0);
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
  const timer = $derived.by(() => {
    const seconds = Math.floor((elapsedMs ?? clock) / 1000);
    return `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
  });

  // Re-runs when `source` changes: a picture that becomes the selected one starts moving.
  $effect(() => {
    if (source === 'still') {
      travel = vuToTravel(-3);
      bars = Array.from({ length: BARS }, (_, i) => still(i));
      return;
    }
    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
    const needle = new Needle();
    const smooth = new Array<number>(BARS).fill(0);
    const seeds = Array.from({ length: BARS }, (_, i) => 1.3 + ((i * 7919) % 13) / 4);
    let last = 0;
    demoStarted = performance.now();
    return onFrame((now) => {
      const dt = last ? (now - last) / 1000 : 0;
      last = now;
      const t = (now - demoStarted) / 1000;
      clock = now - demoStarted;
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
      travel = reduced ? needle.settle(target, dt) : needle.step(target, dt);
      peak = peakDb > -1;
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
  <svg class="flow" class:transcribing viewBox="0 0 200 40" role="img" aria-hidden="true">
    <rect class="pill" x="0.5" y="0.5" width="199" height="39" rx="19.5" />
    {#if transcribing}
      {#each [0, 1, 2] as dot (dot)}
        <circle class="dot" cx={88 + dot * 12} cy="20" r="2.5" style:animation-delay="{dot * 150}ms" />
      {/each}
    {:else}
      {#each bars as height, i (i)}
        {@const h = 2 + height * 24}
        <rect class="bar" x={100 - (BARS * 6 - 3) / 2 + i * 6} y={20 - h / 2} width="3" height={h} rx="1.5" />
      {/each}
      {#if language}<text class="lang" x="178" y="23.5" text-anchor="middle">{language}</text>{/if}
    {/if}
  </svg>
{:else}
  <svg class="analog" class:transcribing viewBox="-8 -8 296 148" role="img" aria-hidden="true">
    <defs>
      <radialGradient id="vu-lamp" cx={PIVOT.x} cy={PIVOT.y} r="150" gradientUnits="userSpaceOnUse">
        <stop offset="0" class="lamp-core" />
        <stop offset="1" class="lamp-edge" />
      </radialGradient>
    </defs>
    <rect class="frame" x="0.5" y="0.5" width="279" height="131" />
    {#each [[0, 0], [280, 0], [0, 132], [280, 132]] as [x, y] (`${x}-${y}`)}
      <path class="tick" d="M {x! - 5.5} {y} H {x! + 5.5} M {x} {y! - 5.5} V {y! + 5.5}" />
    {/each}
    <rect class="face" x="8" y="8" width="264" height="92" />
    <rect class="glow" x="8" y="8" width="264" height="92" fill="url(#vu-lamp)" />
    <path class="scale" d={arc(0, 1, RADIUS)} />
    <path class="red" d={arc(red, 1, RADIUS)} />
    {#each SCALE as [vu, label] (vu)}
      {@const at = vuToTravel(vu)}
      {@const inner = point(at, RADIUS - (label ? 7 : 4))}
      {@const outer = point(at, RADIUS)}
      {@const text = point(at, RADIUS + 9)}
      <line class="mark" class:hot={vu >= 0} x1={inner.x} y1={inner.y} x2={outer.x} y2={outer.y} />
      {#if label}<text class="num" class:hot={vu > 0} x={text.x} y={text.y + 2.5} text-anchor="middle">{vu > 0 ? `+${label}` : label}</text>{/if}
    {/each}
    <text class="vu" x={PIVOT.x} y="68" text-anchor="middle">VU</text>
    <rect class="lamp" class:on={peak} x="256" y="14" width="6" height="6" />
    <line class="needle" x1={PIVOT.x} y1={PIVOT.y} x2={needleTip.x} y2={needleTip.y} />
    <rect class="shroud" x="8" y="100" width="264" height="24" />
    <line class="seam" x1="8" y1="100.5" x2="272" y2="100.5" />
    {#if transcribing}
      <text class="hud" x="18" y="115.5">TRANSCRIBING</text>
      {#each Array.from({ length: 22 }, (_, i) => i) as i (i)}
        <rect class="scan" x={112 + i * 4} y="110" width="2" height="5" style:animation-delay="{i * 40}ms" />
      {/each}
    {:else}
      <circle class="rec" cx="20" cy="112" r="2.5" />
      <text class="hud" x="27" y="115.5">REC {timer}</text>
      <text class="hud dim" x={language ? 226 : 262} y="115.5" text-anchor="end">{engine}</text>
      {#if language}
        <rect class="badge" x="233" y="105.5" width="30" height="13" />
        <text class="hud" x="248" y="115.5" text-anchor="middle">{language}</text>
      {/if}
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
    animation: shimmer 900ms var(--ease) infinite;
  }

  .lang {
    font-family: var(--font-mono);
    font-size: 8px;
    letter-spacing: 0.1em;
    fill: var(--muted);
  }

  .frame {
    fill: var(--bg-raised);
    stroke: var(--line-strong);
  }

  .tick {
    stroke: var(--line-strong);
  }

  .face {
    fill: var(--bg-sunken);
  }

  .lamp-core {
    stop-color: var(--app-amber);
    stop-opacity: 0.08;
    transition: stop-opacity var(--dur-card) ease-out;
  }

  .transcribing .lamp-core {
    stop-opacity: 0.03;
  }

  .lamp-edge {
    stop-color: var(--app-amber);
    stop-opacity: 0;
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

  .num {
    font-family: var(--font-mono);
    font-size: 8px;
    fill: var(--muted);
  }

  .num.hot {
    fill: var(--danger);
  }

  .vu {
    font-family: var(--font-serif);
    font-style: italic;
    font-size: 22px;
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

  .shroud {
    fill: var(--bg-raised);
  }

  .seam {
    stroke: var(--line);
  }

  .hud {
    font-family: var(--font-mono);
    font-size: 8.5px;
    font-weight: 500;
    letter-spacing: 0.12em;
    fill: var(--ink);
  }

  .hud.dim {
    fill: var(--muted);
  }

  .badge {
    fill: var(--bg-sunken);
    stroke: var(--line);
  }

  .rec {
    fill: var(--danger);
    animation: blink 1s steps(1) infinite;
  }

  .scan {
    fill: var(--line-strong);
    animation: scan 880ms linear infinite;
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

  @keyframes scan {
    0%,
    100% {
      fill: var(--line);
    }
    20% {
      fill: var(--ink-strong);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .dot,
    .rec,
    .scan {
      animation: none;
    }
  }
</style>
