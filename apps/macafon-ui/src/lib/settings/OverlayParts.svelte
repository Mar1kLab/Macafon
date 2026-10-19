<script lang="ts">
  // What the hotkey overlay shows (§9.9): one card per part, on when outlined in the accent.
  // Sounds and Dictation keep their own `show_in_overlay` preferences; the rest live in the
  // overlay's preferences.
  import { AudioLines, AudioWaveform, Gauge, Headphones, Layers, Mic, SlidersHorizontal, Speech, SlidersVertical } from '@lucide/svelte';
  import type { Component } from 'svelte';
  import type { OverlayPrefs } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t } from '../i18n';

  let { setup, onchange }: { setup: OverlayPrefs; onchange: (patch: Partial<OverlayPrefs>) => void } = $props();

  type Key = 'volumes' | 'mic_switch' | 'processing_switch' | 'mixer_switch' | 'dictation' | 'sounds' | 'preset' | 'output_device' | 'mic_level';
  interface Part {
    key: Key;
    icon: Component;
    on: boolean;
    fresh?: boolean;
    set: (on: boolean) => void;
  }

  const parts = $derived(
    [
      { key: 'volumes', icon: SlidersVertical, on: setup.volumes, set: (volumes: boolean) => onchange({ volumes }) },
      { key: 'mic_switch', icon: Mic, on: setup.mic_switch, set: (mic_switch: boolean) => onchange({ mic_switch }) },
      { key: 'processing_switch', icon: AudioLines, on: setup.processing_switch, set: (processing_switch: boolean) => onchange({ processing_switch }) },
      { key: 'mixer_switch', icon: SlidersHorizontal, on: setup.mixer_switch, set: (mixer_switch: boolean) => onchange({ mixer_switch }) },
      daemon.dictation && {
        key: 'dictation',
        icon: Speech,
        on: daemon.dictation.prefs.show_in_overlay,
        set: (show_in_overlay: boolean) => daemon.setDictationPrefs({ show_in_overlay }),
      },
      daemon.soundpad && {
        key: 'sounds',
        icon: AudioWaveform,
        on: daemon.soundpad.prefs.show_in_overlay,
        set: (show_in_overlay: boolean) => daemon.setSoundpadPrefs({ show_in_overlay }),
      },
      { key: 'preset', icon: Layers, on: setup.preset, set: (preset: boolean) => onchange({ preset }) },
      { key: 'output_device', icon: Headphones, on: setup.output_device, fresh: true, set: (output_device: boolean) => onchange({ output_device }) },
      { key: 'mic_level', icon: Gauge, on: setup.mic_level, fresh: true, set: (mic_level: boolean) => onchange({ mic_level }) },
    ].filter(Boolean) as Part[],
  );
</script>

<ul class="parts" aria-label={t('settings.overlay.parts')}>
  {#each parts as part (part.key)}
    {@const Icon = part.icon}
    <li>
      <button class:on={part.on} role="switch" aria-checked={part.on} onclick={() => part.set(!part.on)}>
        <Icon size={16} strokeWidth={1.5} />
        <span class="text">
          <span class="name">{t(`settings.overlay.part.${part.key}`)}{#if part.fresh}<small>{t('settings.overlay.new')}</small>{/if}</span>
          <span class="hint">{t(`settings.overlay.partHint.${part.key}`)}</span>
        </span>
        <span class="box" aria-hidden="true"></span>
      </button>
    </li>
  {/each}
</ul>

<style>
  .parts {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 8px;
  }

  button {
    width: 100%;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: start;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--faint);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }

  button:hover {
    border-color: var(--line-strong);
    color: var(--muted);
  }

  button:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 2px;
  }

  button.on {
    border-color: var(--accent);
    color: var(--accent-ink);
  }

  .text {
    display: grid;
    gap: 2px;
  }

  .name {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 13px;
    color: var(--muted);
  }

  .on .name {
    color: var(--ink-strong);
  }

  small {
    font-family: var(--font-mono);
    font-size: 9px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-amber-ink);
  }

  .hint {
    font-size: 12px;
    line-height: 1.4;
    color: var(--faint);
  }

  .box {
    width: 14px;
    height: 14px;
    margin-top: 2px;
    border: 1px solid var(--line-strong);
  }

  .on .box {
    border-color: var(--accent);
  }

  .on .box::after {
    content: '';
    display: block;
    margin: 3px;
    height: 6px;
    background: var(--accent);
  }

  @media (prefers-reduced-motion: reduce) {
    button {
      transition: none;
    }
  }
</style>
