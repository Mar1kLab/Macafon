<script lang="ts">
  // Now playing: pause, stop, repeat and a seekable ruler of the sound's progress (SND-003).
  import { Pause, Play, Repeat, Square } from '@lucide/svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { formatDuration } from './format';
  import SeekBar from './SeekBar.svelte';
  import { tip } from '../components/tooltip.svelte';

  const playback = $derived(daemon.sounddeckPlayback);
  const sound = $derived(playback && daemon.sounddeck?.sounds.find((item) => item.id === playback.sound_id));
</script>

<footer class="transport" class:idle={!playback}>
  <div class="buttons">
    <button
      class="btn icon"
      class:on={playback && !playback.paused}
      disabled={!playback}
      aria-label={playback?.paused ? t('sounddeck.resume') : t('sounddeck.pause')}
      {@attach tip(playback?.paused ? t('sounddeck.resume') : t('sounddeck.pause'))}
      onclick={() => daemon.run(daemon.client.call('sounddeck.set_paused', { paused: null }))}
    >
      {#if !playback || playback.paused}<Play size={14} strokeWidth={1.5} />{:else}<Pause size={14} strokeWidth={1.5} />{/if}
    </button>
    <button class="btn icon" disabled={!playback} aria-label={t('sounddeck.stop')} {@attach tip(t('sounddeck.stop'))} onclick={() => daemon.stopSound()}>
      <Square size={13} strokeWidth={1.5} />
    </button>
    <button
      class="btn icon"
      class:on={playback?.looping}
      disabled={!playback}
      aria-pressed={playback?.looping ?? false}
      aria-label={t('sounddeck.loop')}
      {@attach tip(t('sounddeck.loop'))}
      onclick={() => playback && daemon.run(daemon.client.call('sounddeck.set_looping', { looping: !playback.looping }))}
    >
      <Repeat size={14} strokeWidth={1.5} />
    </button>
  </div>

  <div class="now" aria-live="polite">
    {#if playback}
      <span class="hud-label">{playback.loading ? t('sounddeck.loading') : (sound?.category ?? t('sounddeck.uncategorized'))}</span>
      <b {@attach tip(sound?.name)}>{sound?.name ?? playback.sound_id}</b>
    {:else}
      <span class="hud-label">{t('sounddeck.nothingPlaying')}</span>
    {/if}
  </div>

  <SeekBar />

  <span class="time value">
    {formatDuration(playback?.position_ms ?? 0)}<span class="total">&nbsp;/ {formatDuration(playback?.duration_ms ?? null)}</span>
  </span>
</footer>

<style>
  .transport {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    height: 64px;
    padding: 0 24px;
    border-top: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .buttons {
    display: flex;
    gap: var(--space-2);
  }

  .now {
    width: 220px;
    flex: none;
    display: grid;
    min-width: 0;
  }

  .now .hud-label {
    font-size: var(--text-2xs);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .now b {
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--ink-strong);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .time {
    width: 112px;
    flex: none;
    text-align: right;
  }

  .total {
    color: var(--muted);
  }
</style>
