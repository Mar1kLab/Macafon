<script lang="ts">
  // 02 / MIXER (PRD §9.4): output device in the header (OUT-001), channel strips with app
  // chips moved by drag & drop (APP-004), a new-channel tile (CH-002), the Master (CH-006) and
  // a drawer with EQ / FX / channel settings. Off, the screen offers to turn the mixer on
  // (CH-009).
  import { tick, untrack } from 'svelte';
  import { Plus } from '@lucide/svelte';
  import ChannelStrip from '../lib/mixer/ChannelStrip.svelte';
  import MasterStrip from '../lib/mixer/MasterStrip.svelte';
  import OutputDevicePanel from '../lib/mixer/OutputDevicePanel.svelte';
  import ChannelDrawer, { type Panel } from '../lib/mixer/ChannelDrawer.svelte';
  import Dropdown from '../lib/components/Dropdown.svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import Toggle from '../lib/components/Toggle.svelte';
  import Ticks from '../lib/components/Ticks.svelte';
  import { channelIcon } from '../lib/mixer/icons';
  import { daemon } from '../lib/state';
  import { CHANNEL_COLORS, MAX_CHANNELS } from '../lib/ipc/dsp';
  import { t, type MessageKey } from '../lib/i18n';

  const PREVIEWS: Record<string, MessageKey> = {
    main: 'mixer.previews.main',
    voice: 'mixer.previews.voice',
    music: 'mixer.previews.music',
  };
  const mixer = $derived(daemon.mixer);
  let drawer = $state<{ id: string; panel: Panel } | null>(null);
  let dragging = $state<string | null>(null);
  let reordering = $state<string | null>(null);
  let showInactive = $state(true);
  let flashing = $state(new Set<string>());
  let creating = $state(false);
  let outputPanel = $state<string | null>(null);
  const outputControl = $derived(daemon.outputControls.find((item) => item.output !== null && item.output === mixer?.output));
  let newName = $state('');
  let newColor = $state('orange');

  /** WIN-CH-006: explain once why channels have no EQ/FX here. */
  const FX_NOTE_KEY = 'macafon.mixer.channelFxNote';
  $effect(() => {
    if (mixer?.enabled && !daemon.channelFx && localStorage.getItem(FX_NOTE_KEY) !== 'shown') {
      localStorage.setItem(FX_NOTE_KEY, 'shown');
      daemon.toast(t('mixer.channelFxLinux'), { message: t('mixer.channelFxLinuxBody') });
    }
  });

  const drawerChannel = $derived(drawer && mixer?.channels.find((c) => c.id === drawer!.id));
  const outputOptions = $derived([
    { value: null as string | null, label: t('mixer.followDefault') },
    ...(mixer?.outputs ?? []).map((output) => ({ value: output.name as string | null, label: output.description })),
  ]);
  const outputValue = $derived(mixer?.output_priority[0] ?? null);
  const current = $derived(mixer?.outputs.find((output) => output.name === mixer.output));

  function appsOf(id: string) {
    return (mixer?.apps ?? []).filter((app) => app.channel === id && (showInactive || app.streams > 0));
  }

  // A new app flashes in its channel colour (§9.4).
  let known: Set<string> | null = null;
  $effect(() => {
    const keys = new Set((mixer?.apps ?? []).filter((app) => app.streams > 0).map((app) => app.key));
    untrack(() => {
      if (known) {
        const fresh = [...keys].filter((key) => !known!.has(key));
        if (fresh.length) {
          flashing = new Set([...flashing, ...fresh]);
          setTimeout(() => (flashing = new Set([...flashing].filter((key) => !fresh.includes(key)))), 900);
        }
      }
      known = keys;
    });
  });

  /** CH-007: the new order is shown at once and confirmed by the daemon. */
  function reorder(ids: string[]) {
    const current = daemon.mixer;
    if (!current || ids.join() === current.channels.map((c) => c.id).join()) return;
    daemon.mixer = { ...current, channels: ids.map((id) => current.channels.find((c) => c.id === id)!) };
    void daemon.setMixer(daemon.client.call('channel.reorder', { ids }));
  }

  function dropChannel(target: string, after: boolean) {
    const moving = reordering;
    reordering = null;
    if (!mixer || !moving || moving === target) return;
    const ids = mixer.channels.map((c) => c.id).filter((id) => id !== moving);
    const index = ids.indexOf(target) + (after ? 1 : 0);
    ids.splice(index, 0, moving);
    reorder(ids);
  }

  async function shift(id: string, delta: number) {
    if (!mixer) return;
    const ids = mixer.channels.map((c) => c.id);
    const from = ids.indexOf(id);
    const to = from + delta;
    if (to < 0 || to >= ids.length) return;
    ids.splice(from, 1);
    ids.splice(to, 0, id);
    reorder(ids);
    // Keep the keyboard on the moved handle.
    await tick();
    document.querySelector<HTMLElement>(`section[data-channel="${id}"] .handle`)?.focus();
  }

  function openCreate() {
    const used = new Set(mixer?.channels.map((c) => c.color));
    newColor = CHANNEL_COLORS.find((color) => !used.has(color)) ?? 'orange';
    newName = '';
    creating = true;
  }

  async function create() {
    const name = newName.trim();
    if (!name) return;
    creating = false;
    await daemon.setMixer(daemon.client.call('channel.create', { name, color: newColor, icon: null }));
  }
</script>

{#if mixer}
  <div class="screen">
    <header class="top">
      <h1 class="hud-label">02 / {t('mixer.section')}</h1>
      <label class="switch">
        <span class="hud-label">{t('mixer.enabled')}</span>
        <Toggle checked={mixer.enabled} label={t('mixer.enabled')} onchange={(enabled) => daemon.setMixer(daemon.client.call('mixer.set_enabled', { enabled }))} />
      </label>
      <span class="spacer"></span>
      {#if mixer.enabled}
        <label class="switch" title={t('mixer.showInactiveHint')}>
          <span class="hud-label">{t('mixer.showInactive')}</span>
          <Toggle checked={showInactive} label={t('mixer.showInactive')} onchange={(on) => (showInactive = on)} />
        </label>
      {/if}
      {#if mixer.enabled}
      <span class="hud-label">{t('mixer.output')}</span>
      <Dropdown
        label={t('mixer.output')}
        options={outputOptions}
        value={outputValue}
        width="260px"
        onchange={(device) => daemon.setMixer(daemon.client.call('mixer.set_output', { device }))}
      >
        {#snippet button()}
          <span>{current?.description ?? t('mixer.noOutput')}</span>
        {/snippet}
      </Dropdown>
      {#if outputControl}
        <button class="btn small" onclick={() => (outputPanel = outputControl!.id)}>{t('outputDevice.open')}</button>
      {/if}
      {/if}
    </header>

    {#if !mixer.enabled}
      <div class="off">
        <section class="intro">
          <Ticks />
          <span class="hud-label">{t('mixer.offLabel')}</span>
          <h2>{t('mixer.offTitle')} <em>{t('mixer.offAccent')}</em>?</h2>
          <p>{t('mixer.offBody')}</p>
          <ul class="preview" aria-hidden="true">
            {#each mixer.channels as channel (channel.id)}
              {@const Icon = channelIcon(channel.icon)}
              <li style:--c="var(--app-{channel.color})" style:--c-ink="var(--app-{channel.color}-ink)">
                <Icon size={14} strokeWidth={1.5} /><b>{channel.name}</b>
                {#if PREVIEWS[channel.id]}<small>{t(PREVIEWS[channel.id]!)}</small>{/if}
              </li>
            {/each}
          </ul>
          <p class="note">{daemon.windows ? t('mixer.offNoteWindows') : t('mixer.offNote')}</p>
          <button class="btn invert" onclick={() => daemon.setMixer(daemon.client.call('mixer.set_enabled', { enabled: true }))}>{t('mixer.enable')}</button>
        </section>
      </div>
    {:else}
      <div class="board">
        <div class="channels">
          {#each mixer.channels as channel (channel.id)}
            <ChannelStrip
              {channel}
              channels={mixer.channels}
              apps={appsOf(channel.id)}
              isDefault={channel.id === mixer.default_channel}
              defaultName={mixer.channels.find((c) => c.id === mixer.default_channel)?.name ?? ''}
              {dragging}
              {flashing}
              onpanel={(panel) => (drawer = { id: channel.id, panel })}
              ondragstart={(key) => (dragging = key)}
              ondragend={() => (dragging = null)}
              {reordering}
              onreorderstart={() => (reordering = channel.id)}
              onreorderend={() => (reordering = null)}
              ondropchannel={(after) => dropChannel(channel.id, after)}
              onshift={(delta) => shift(channel.id, delta)}
            />
          {/each}
          {#if mixer.channels.length < MAX_CHANNELS}
            <button class="new" onclick={openCreate}>
              <Plus size={16} strokeWidth={1.5} />
              <span class="hud-label">{t('mixer.newChannel')}</span>
            </button>
          {/if}
          <span class="spacer"></span>
          <MasterStrip {mixer} />
        </div>
        {#if drawer && drawerChannel}
          <ChannelDrawer
            channel={drawerChannel}
            panel={drawer.panel}
            isDefault={drawerChannel.id === mixer.default_channel}
            onpanel={(panel) => drawer && (drawer = { ...drawer, panel })}
            onclose={() => (drawer = null)}
          />
        {/if}
      </div>
      <p class="footnote">{t('mixer.footnote')}</p>
    {/if}
  </div>
{:else}
  <div class="loading" role="status"><span class="hud-label">{t('daemon.loading')}</span></div>
{/if}

<Dialog open={creating} title={t('mixer.newChannel')} onclose={() => (creating = false)}>
  <form
    class="create"
    onsubmit={(event) => {
      event.preventDefault();
      void create();
    }}
  >
    <label class="field">
      <span class="hud-label">{t('mixer.name')}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input" bind:value={newName} maxlength={32} placeholder={t('mixer.namePlaceholder')} autofocus />
    </label>
    <div class="field">
      <span class="hud-label">{t('mixer.color')}</span>
      <div class="swatches" role="radiogroup" aria-label={t('mixer.color')}>
        {#each CHANNEL_COLORS as color (color)}
          <button type="button" role="radio" aria-checked={newColor === color} aria-label={color} class:selected={newColor === color} style:--s="var(--app-{color})" onclick={() => (newColor = color)}></button>
        {/each}
      </div>
    </div>
  </form>
  {#snippet actions()}
    <button class="btn" onclick={() => (creating = false)}>{t('common.cancel')}</button>
    <button class="btn invert" disabled={!newName.trim()} onclick={create}>{t('mixer.create')}</button>
  {/snippet}
</Dialog>

{#if outputPanel}
  <OutputDevicePanel id={outputPanel} onclose={() => (outputPanel = null)} />
{/if}

<style>
  .screen {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 10px 14px;
    min-height: 59px;
    padding: 14px 24px;
    border-bottom: 1px solid var(--line);
  }

  h1 {
    color: var(--ink);
    white-space: nowrap;
  }

  .switch {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }

  .board {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .channels {
    height: 100%;
    display: flex;
    gap: 12px;
    padding: 20px 24px;
    overflow-x: auto;
  }

  .new {
    flex: none;
    width: 120px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 1px dashed var(--line-strong);
    color: var(--muted);
    align-self: flex-start;
    height: 120px;
    transition: border-color var(--dur) var(--ease);
  }

  .new:hover {
    border-color: var(--ink);
    color: var(--ink);
  }

  .footnote {
    padding: 8px 24px 12px;
    font-size: 12px;
    color: var(--faint);
  }

  .off {
    flex: 1;
    display: flex;
    align-items: safe center;
    justify-content: center;
    overflow-y: auto;
    padding: 24px;
  }

  .intro {
    position: relative;
    display: grid;
    gap: 16px;
    justify-items: start;
    width: 640px;
    padding: 32px;
    border: 1px solid var(--line);
    background: var(--bg-raised);
  }

  h2 {
    font-family: var(--font-display);
    font-size: 36px;
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1.1;
    color: var(--ink-strong);
  }

  em {
    font-family: var(--font-serif);
    font-weight: 400;
    letter-spacing: 0;
  }

  .intro p {
    color: var(--ink);
  }

  .intro .note {
    font-size: 13px;
    color: var(--muted);
  }

  .preview {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .preview li {
    display: grid;
    align-content: start;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--line);
    border-top: 3px solid var(--c);
    color: var(--c-ink);
  }

  .preview b {
    font-family: var(--font-display);
    font-size: 16px;
    color: var(--ink-strong);
  }

  .preview small {
    font-size: 12px;
    color: var(--muted);
  }

  .loading {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .create {
    display: grid;
    gap: 16px;
  }

  .field {
    display: grid;
    gap: 8px;
  }

  .input {
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
    color: var(--ink);
    font: inherit;
  }

  .input:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 2px;
  }

  .swatches {
    display: flex;
    gap: 6px;
  }

  .swatches button {
    width: 28px;
    height: 28px;
    background: var(--s);
    border: 2px solid transparent;
  }

  .swatches button.selected {
    border-color: var(--ink-strong);
  }
</style>
