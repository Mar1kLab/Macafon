<script lang="ts">
  // The seekable ruler of the playing sound's progress (SND-003): drag or click to seek, ← →
  // one second, Page Up/Down five, Home to the start. Shared by the transport and the overlay.
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { formatDuration } from './format';

  let { compact = false }: { compact?: boolean } = $props();

  const playback = $derived(daemon.sounddeckPlayback);
  const progress = $derived(playback && playback.duration_ms > 0 ? Math.min(playback.position_ms / playback.duration_ms, 1) : 0);
  const ticks = Array.from({ length: 21 }, (_, i) => i);

  let track: HTMLDivElement | undefined = $state();
  let seeking = $state(false);

  function seekTo(event: PointerEvent) {
    if (!playback || !track) return;
    const rect = track.getBoundingClientRect();
    const fraction = Math.min(Math.max((event.clientX - rect.left) / rect.width, 0), 1);
    void daemon.run(daemon.client.call('sounddeck.seek', { position_ms: Math.round(fraction * playback.duration_ms) }));
  }

  function onkeydown(event: KeyboardEvent) {
    if (!playback) return;
    const step = { ArrowRight: 1000, ArrowUp: 1000, ArrowLeft: -1000, ArrowDown: -1000, PageUp: 5000, PageDown: -5000 }[event.key];
    const target = event.key === 'Home' ? 0 : step === undefined ? null : playback.position_ms + step;
    if (target === null) return;
    event.preventDefault();
    const position_ms = Math.round(Math.min(Math.max(target, 0), playback.duration_ms));
    void daemon.run(daemon.client.call('sounddeck.seek', { position_ms }));
  }
</script>

<div
  class="track"
  class:idle={!playback}
  class:compact
  bind:this={track}
  role="slider"
  tabindex={playback ? 0 : -1}
  aria-label={t('sounddeck.position')}
  aria-valuemin={0}
  aria-valuemax={Math.round((playback?.duration_ms ?? 0) / 1000)}
  aria-valuenow={Math.round((playback?.position_ms ?? 0) / 1000)}
  aria-valuetext={`${formatDuration(playback?.position_ms ?? 0)} / ${formatDuration(playback?.duration_ms ?? 0)}`}
  aria-disabled={!playback}
  onpointerdown={(event) => {
    if (!playback || event.button !== 0) return;
    track?.setPointerCapture(event.pointerId);
    seeking = true;
    seekTo(event);
  }}
  onpointermove={(event) => seeking && seekTo(event)}
  onpointerup={(event) => {
    seeking = false;
    track?.releasePointerCapture(event.pointerId);
  }}
  {onkeydown}
>
  <span class="fill" style:width="{progress * 100}%"></span>
  {#each ticks as tick (tick)}
    <span class="tick" class:major={tick % 5 === 0} style:left="{tick * 5}%"></span>
  {/each}
  <span class="head" style:left="{progress * 100}%"></span>
</div>

<style>
  .track {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 22px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    cursor: pointer;
    touch-action: none;
  }

  .track.compact {
    height: 18px;
  }

  .idle {
    cursor: default;
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    opacity: 0.35;
  }

  .tick {
    position: absolute;
    bottom: 0;
    width: 1px;
    height: 5px;
    background: var(--faint);
  }

  .tick.major {
    height: 9px;
    background: var(--line-strong);
  }

  .compact .tick {
    height: 4px;
  }

  .compact .tick.major {
    height: 7px;
  }

  .head {
    position: absolute;
    top: -1px;
    bottom: -1px;
    width: 2px;
    margin-left: -1px;
    background: var(--accent-ink);
  }

  .idle .head {
    display: none;
  }
</style>
