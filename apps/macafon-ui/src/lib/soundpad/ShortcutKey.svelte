<script lang="ts">
  // One shortcut slot of an action, recorded by pressing the keys (HK-002), as in Settings →
  // Hotkeys: conflicts inside Macafon are caught before the daemon is asked.
  import { X } from '@lucide/svelte';
  import { daemon } from '../state';
  import { t } from '../i18n';
  import { DaemonError } from '../ipc/client';
  import { displayShortcut, recordShortcut } from '../settings/shortcut';

  interface Props {
    action: string;
    secondary?: boolean;
    /** What the action does, for labels and conflict messages. */
    label: string;
  }

  let { action, secondary = false, label }: Props = $props();

  const binding = $derived(daemon.hotkeys?.bindings.find((item) => item.action === action));
  const shortcut = $derived((secondary ? binding?.secondary_shortcut : binding?.shortcut) ?? null);
  const registered = $derived((secondary ? binding?.secondary_error : binding?.error) ?? null);
  const slotName = $derived(t(secondary ? 'settings.hotkeys.secondary' : 'settings.hotkeys.primary'));

  let recording = $state(false);
  let preview = $state('');
  let problem = $state<string | null>(null);
  const sides = new Set<string>();

  function start() {
    sides.clear();
    preview = '';
    problem = null;
    recording = true;
  }

  function stop() {
    sides.clear();
    recording = false;
    preview = '';
  }

  async function save(next: string | null) {
    try {
      daemon.hotkeys = await daemon.client.call('hotkeys.set', { action, shortcut: next, secondary });
      problem = null;
    } catch (error) {
      problem = error instanceof DaemonError ? error.message : String(error);
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (!recording || event.key === 'Tab') return;
    event.preventDefault();
    event.stopPropagation();
    const plain = !(event.ctrlKey || event.altKey || event.shiftKey || event.metaKey);
    if (event.code === 'Escape' && plain) return stop();
    if (/^(Control|Alt)(Left|Right)$/.test(event.code)) sides.add(event.code);
    const result = recordShortcut(event, daemon.windows ? sides : undefined);
    if (result.kind === 'partial') {
      preview = result.preview;
      return;
    }
    if (result.kind === 'needs_modifier') {
      problem = t('settings.hotkeys.needsModifier', { shortcut: result.preview });
      return;
    }
    if (result.kind === 'unsupported') {
      problem = t('settings.hotkeys.unsupported', { shortcut: result.preview });
      return;
    }
    stop();
    const slot = secondary ? `${action}#secondary` : action;
    const taken = daemon.hotkeys?.bindings.find(
      (b) => (b.action !== slot && b.shortcut === result.shortcut) || (`${b.action}#secondary` !== slot && b.secondary_shortcut === result.shortcut),
    );
    if (taken) {
      problem = t('settings.hotkeys.conflict', { shortcut: displayShortcut(result.shortcut), action: taken.action });
      return;
    }
    void save(result.shortcut);
  }

  function onkeyup(event: KeyboardEvent) {
    if (!recording) return;
    sides.delete(event.code);
    const result = recordShortcut(event, daemon.windows ? sides : undefined);
    if (result.kind === 'partial') preview = result.preview;
  }
</script>

<div class="slot">
  <div class="row">
    <button
      class="key"
      class:recording
      class:empty={!shortcut}
      aria-label={`${t('settings.hotkeys.record', { action: label })} / ${slotName}`}
      onclick={() => (recording ? stop() : start())}
      {onkeydown}
      {onkeyup}
      onblur={() => recording && stop()}
    >
      {recording ? preview || t('settings.hotkeys.press') : shortcut ? displayShortcut(shortcut) : slotName}
    </button>
    <button class="btn icon ghost small" aria-label={`${t('settings.hotkeys.clear', { action: label })} / ${slotName}`} disabled={!shortcut} onclick={() => save(null)}>
      <X size={13} strokeWidth={1.5} />
    </button>
  </div>
  {#if problem ?? registered}
    <p class="problem" role="alert">{problem ?? registered}</p>
  {:else if recording}
    <p class="note">{t('settings.hotkeys.recordHint')}</p>
  {/if}
</div>

<style>
  .slot {
    display: grid;
    gap: 4px;
    min-width: 0;
  }

  .row {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  .key {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 5px 10px;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--ink);
    text-align: center;
    background: var(--bg-sunken);
    border: 1px solid var(--line-strong);
    transition: border-color var(--dur) var(--ease);
  }

  .key:hover {
    border-color: var(--ink);
  }

  .key.empty {
    color: var(--faint);
  }

  .key.recording {
    color: var(--accent-ink);
    border-color: var(--accent);
  }

  .problem,
  .note {
    font-size: 12px;
  }

  .problem {
    color: var(--danger);
  }

  .note {
    color: var(--muted);
  }
</style>
