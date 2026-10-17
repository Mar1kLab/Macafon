<script lang="ts">
  // Quick panel volume (Android-like): a thick pill that fills with the channel colour, its
  // name and value inside. Drag or click anywhere on it, scroll over it, or focus it and use
  // ← → (5 %), Page Up/Down (10 %), Home/End. Muted: the fill fades and the value says so.
  interface Props {
    value: number;
    label: string;
    name: string;
    color: string;
    muted?: boolean;
    mutedText: string;
    onchange: (value: number) => void;
  }

  let { value, label, name, color, muted = false, mutedText, onchange }: Props = $props();
  let track: HTMLDivElement | undefined = $state();
  let dragging = $state(false);

  const clamp = (v: number) => Math.min(100, Math.max(0, Math.round(v)));

  function fromPointer(event: PointerEvent) {
    if (!track) return;
    const rect = track.getBoundingClientRect();
    onchange(clamp(((event.clientX - rect.left) / rect.width) * 100));
  }

  function onkeydown(event: KeyboardEvent) {
    const step: Record<string, number> = {
      ArrowRight: 5,
      ArrowUp: 5,
      ArrowLeft: -5,
      ArrowDown: -5,
      PageUp: 10,
      PageDown: -10,
    };
    let next: number | null = null;
    if (event.key in step) next = value + step[event.key]!;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = 100;
    if (next === null) return;
    event.preventDefault();
    onchange(clamp(next));
  }

  function onwheel(event: WheelEvent) {
    event.preventDefault();
    onchange(clamp(value + (event.deltaY < 0 ? 5 : -5)));
  }
</script>

<div
  class="pill"
  class:muted
  class:dragging
  bind:this={track}
  role="slider"
  tabindex="0"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={Math.round(value)}
  aria-valuetext={muted ? mutedText : `${Math.round(value)}%`}
  style:--c="var(--app-{color})"
  style:--fill="{value}%"
  {onkeydown}
  {onwheel}
  onpointerdown={(event) => {
    if (event.button !== 0) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    dragging = true;
    fromPointer(event);
  }}
  onpointermove={(event) => dragging && fromPointer(event)}
  onpointerup={() => (dragging = false)}
  onpointercancel={() => (dragging = false)}
>
  <span class="fill"></span>
  <span class="name">{name}</span>
  <span class="value">{muted ? mutedText : `${Math.round(value)}%`}</span>
</div>

<style>
  .pill {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    height: 40px;
    padding: 0 16px;
    overflow: hidden;
    /* Android and Windows 11 controls are round; an owner exception like the hotkey popup. */
    border-radius: 20px;
    background: var(--bg-sunken);
    border: 1px solid var(--line);
    cursor: pointer;
    touch-action: none;
    user-select: none;
    outline: none;
  }

  .pill:focus-visible {
    border-color: var(--ink);
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    width: var(--fill);
    background: var(--c);
    transition:
      width var(--dur) var(--ease),
      opacity var(--dur) var(--ease);
  }

  .dragging .fill {
    transition: opacity var(--dur) var(--ease);
  }

  .muted .fill {
    opacity: 0.25;
  }

  .name,
  .value {
    position: relative;
    font-size: 13px;
    color: var(--ink-strong);
    white-space: nowrap;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 600;
  }

  .value {
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .muted .value {
    color: var(--danger);
  }

  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
  }
</style>
