<script lang="ts">
  // Now playing: pause, stop, repeat and a seekable ruler of the sound's progress (SND-003).
  import { Pause, Play, Repeat, Square } from '@lucide/svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { formatDuration } from './format';
  import SeekBar from './SeekBar.svelte';

  const playback = $derived(daemon.soundpadPlayback);
  const sound = $derived(playback && daemon.soundpad?.sounds.find((item) => item.id === playback.sound_id));
</script>

<footer class="transport" class:idle={!playback}>
  <div class="buttons">
    <button
      class="btn icon"
      class:on={playback && !playback.paused}
      disabled={!playback}
      aria-label={playback?.paused ? t('soundpad.resume') : t('soundpad.pause')}
      title={playback?.paused ? t('soundpad.resume') : t('soundpad.pause')}
      onclick={() => daemon.run(daemon.client.call('soundpad.set_paused', { paused: null }))}
    >
      {#if !playback || playback.paused}<Play size={14} strokeWidth={1.5} />{:else}<Pause size={14} strokeWidth={1.5} />{/if}
    </button>
    <button class="btn icon" disabled={!playback} aria-label={t('soundpad.stop')} title={t('soundpad.stop')} onclick={() => daemon.stopSound()}>
      <Square size={13} strokeWidth={1.5} />
    </button>
    <button
      class="btn icon"
      class:on={playback?.looping}
      disabled={!playback}
      aria-pressed={playback?.looping ?? false}
      aria-label={t('soundpad.loop')}
      title={t('soundpad.loop')}
      onclick={() => playback && daemon.run(daemon.client.call('soundpad.set_looping', { looping: !playback.looping }))}
    >
      <Repeat size={14} strokeWidth={1.5} />
    </button>
  </div>

  <div class="now" aria-live="polite">
    {#if playback}
      <span class="hud-label">{playback.loading ? t('soundpad.loading') : (sound?.category ?? t('soundpad.uncategorized'))}</span>
      <b title={sound?.name}>{sound?.name ?? playback.sound_id}</b>
    {:else}
      <span class="hud-label">{t('soundpad.nothingPlaying')}</span>
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
    gap: 16px;
    height: 64px;
    padding: 0 24px;
    border-top: 1px solid var(--line);
    background: var(--bg-raised);
  }

  .buttons {
    display: flex;
    gap: 6px;
  }

  .now {
    width: 220px;
    flex: none;
    display: grid;
    min-width: 0;
  }

  .now .hud-label {
    font-size: 10px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .now b {
    font-family: var(--font-display);
    font-size: 15px;
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
