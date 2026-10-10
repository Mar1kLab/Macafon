<script lang="ts">
  // Presets (§6.3.10): built-in are read-only (Save → "Save as new"), user presets can be
  // updated, renamed, duplicated and deleted. Import/export need file dialogs (next round).
  import { Copy, Pencil, Trash2 } from '@lucide/svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import PromptDialog from '../components/PromptDialog.svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import type { MicState, PresetInfo } from '../ipc/protocol';

  let { mic }: { mic: MicState } = $props();

  let prompt = $state<{ mode: 'save' | 'rename'; preset?: PresetInfo } | null>(null);

  const current = $derived(daemon.presets.find((preset) => preset.id === mic.preset));
  const options = $derived(
    daemon.presets.map((preset) => ({
      value: preset.id as string | null,
      label: preset.name,
      group: preset.builtin ? t('presets.builtin') : t('presets.user'),
    })),
  );

  async function save() {
    if (current && !current.builtin) {
      await daemon.run(daemon.client.call('presets.update', { id: current.id }));
      void daemon.setMic(daemon.client.call('mic.get'));
    } else {
      prompt = { mode: 'save' };
    }
  }

  async function submit(name: string) {
    const mode = prompt;
    prompt = null;
    if (mode?.mode === 'rename' && mode.preset) {
      await daemon.run(daemon.client.call('presets.rename', { id: mode.preset.id, name }));
    } else {
      const saved = await daemon.run(daemon.client.call('presets.save', { name }));
      if (saved) void daemon.setMic(daemon.client.call('mic.get'));
    }
  }
</script>

<div class="presets">
  <Dropdown
    label={t('mic.preset')}
    {options}
    value={mic.preset}
    width="180px"
    onchange={(id) => id && daemon.selectPreset(id)}
  >
    {#snippet button()}
      <span class="name">{current?.name ?? '—'}</span>
    {/snippet}
    {#snippet option(item)}
      <span class="option-name">{item.label}</span>
    {/snippet}
    {#snippet footer(close)}
      {#if current}
        <button class="btn small" onclick={() => { close(); void daemon.run(daemon.client.call('presets.duplicate', { id: current.id })); }}>
          <Copy size={12} strokeWidth={1.5} />{t('presets.duplicate')}
        </button>
        {#if !current.builtin}
          <button class="btn small" onclick={() => { close(); prompt = { mode: 'rename', preset: current }; }}>
            <Pencil size={12} strokeWidth={1.5} />{t('presets.rename')}
          </button>
          <button class="btn small" onclick={() => { close(); void daemon.run(daemon.client.call('presets.delete', { id: current.id })); }}>
            <Trash2 size={12} strokeWidth={1.5} />{t('presets.delete')}
          </button>
        {/if}
      {/if}
    {/snippet}
  </Dropdown>
  <!-- Save appears only when there is something to save. -->
  {#if mic.preset_modified || !current}
    {#if mic.preset_modified}<span class="badge modified">{t('mic.modified')}</span>{/if}
    <button class="btn" onclick={save}>
      {current && !current.builtin ? t('mic.save') : t('mic.saveAs')}
    </button>
  {/if}
</div>

<PromptDialog
  open={prompt !== null}
  title={prompt?.mode === 'rename' ? t('presets.renameTitle') : t('mic.saveAs')}
  label={t('mic.savePrompt')}
  initial={prompt?.mode === 'rename' ? (prompt.preset?.name ?? '') : current ? t('presets.copyName', { name: current.name }) : ''}
  confirm={t('mic.save')}
  onsubmit={submit}
  onclose={() => (prompt = null)}
/>

<style>
  .presets {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .name,
  .option-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .modified {
    color: var(--app-amber-ink);
  }
</style>
