<script lang="ts">
  // Settings → Hotkeys (HK-002, HK-004): every action with its shortcut, recorded by pressing
  // the keys; conflicts inside Macafon are caught before the daemon is asked. Without a global
  // shortcut service the CLI commands are shown for the desktop keyboard settings instead.
  import { Copy, X } from '@lucide/svelte';
  import { daemon } from '../state';
  import { t, type MessageKey } from '../i18n';
  import { DaemonError } from '../ipc/client';
  import type { HotkeyBinding } from '../ipc/protocol';
  import { displayShortcut, recordShortcut } from './shortcut';

  const hotkeys = $derived(daemon.hotkeys);
  const unavailable = $derived(hotkeys?.backend === 'unavailable');

  /** Action being recorded and the modifiers held so far. */
  let recording = $state<string | null>(null);
  let preview = $state('');
  /** Inline problem per action: bad key, conflict, daemon refusal. */
  let problems = $state<Record<string, string>>({});

  function label(action: string): string {
    const kind = action.split('.')[0]!;
    return t(`settings.hotkeys.actions.${kind}` as MessageKey);
  }

  function fullLabel(action: string): string {
    const channel = action.split('.')[1];
    if (!channel) return label(action);
    return `${channelName(channel)}: ${label(action)}`;
  }

  function channelName(id: string): string {
    return daemon.mixer?.channels.find((c) => c.id === id)?.name ?? id;
  }

  /** The quick panel, microphone actions, then one group per channel in mixer order. */
  const groups = $derived.by(() => {
    const result: { id: string; title: string; bindings: HotkeyBinding[] }[] = [];
    for (const binding of hotkeys?.bindings ?? []) {
      const channel = binding.action.split('.')[1];
      const general = binding.action === 'quick_panel';
      const id = channel ?? (general ? 'general' : '');
      let group = result.find((g) => g.id === id);
      if (!group) {
        const title = channel
          ? t('settings.hotkeys.channel', { name: channelName(channel) })
          : general
            ? t('settings.hotkeys.general')
            : t('settings.hotkeys.microphone');
        group = { id, title, bindings: [] };
        result.push(group);
      }
      group.bindings.push(binding);
    }
    return result;
  });

  function setProblem(action: string, message: string | null) {
    const { [action]: _, ...rest } = problems;
    problems = message ? { ...rest, [action]: message } : rest;
  }

  function start(action: string) {
    recording = action;
    preview = '';
    setProblem(action, null);
  }

  function stop() {
    recording = null;
    preview = '';
  }

  async function save(action: string, shortcut: string | null) {
    try {
      const next = await daemon.client.call('hotkeys.set', { action, shortcut });
      daemon.hotkeys = next;
      setProblem(action, null);
    } catch (error) {
      setProblem(action, error instanceof DaemonError ? error.message : String(error));
    }
  }

  function onkeydown(action: string, event: KeyboardEvent) {
    if (recording !== action) return;
    // Tab still moves focus; recording ends on blur.
    if (event.key === 'Tab') return;
    event.preventDefault();
    event.stopPropagation();
    const plain = !(event.ctrlKey || event.altKey || event.shiftKey || event.metaKey);
    if (event.code === 'Escape' && plain) {
      stop();
      return;
    }
    const result = recordShortcut(event);
    if (result.kind === 'partial') {
      preview = result.preview;
      return;
    }
    if (result.kind === 'needs_modifier') {
      setProblem(action, t('settings.hotkeys.needsModifier', { shortcut: result.preview }));
      return;
    }
    if (result.kind === 'unsupported') {
      setProblem(action, t('settings.hotkeys.unsupported', { shortcut: result.preview }));
      return;
    }
    stop();
    const taken = hotkeys?.bindings.find((b) => b.action !== action && b.shortcut === result.shortcut);
    if (taken) {
      setProblem(action, t('settings.hotkeys.conflict', { shortcut: displayShortcut(result.shortcut), action: fullLabel(taken.action) }));
      return;
    }
    void save(action, result.shortcut);
  }

  function onkeyup(action: string, event: KeyboardEvent) {
    if (recording !== action) return;
    const result = recordShortcut(event);
    if (result.kind === 'partial') preview = result.preview;
  }

  async function copy(command: string) {
    try {
      await navigator.clipboard.writeText(command);
      daemon.toast(t('settings.hotkeys.copied'));
    } catch (error) {
      daemon.fail(error);
    }
  }
</script>

{#if hotkeys}
  <div class="hotkeys">
    <div class="status">
      <div>
        <p class="name">{t('settings.hotkeys.backend')}</p>
        <p class="hint">
          {unavailable
            ? t('settings.hotkeys.unavailableHint')
            : hotkeys.backend === 'portal'
              ? t('settings.hotkeys.portalHint')
              : t('settings.hotkeys.x11Hint')}
        </p>
      </div>
      <span class="badge" class:off={unavailable}>{t(`settings.hotkeys.backends.${hotkeys.backend}`)}</span>
    </div>

    {#each groups as group (group.id)}
      <div class="group">
        <h3 class="hud-label">{group.title}</h3>
        <ul>
          {#each group.bindings as binding (binding.action)}
            {@const action = binding.action}
            {@const problem = problems[action] ?? binding.error}
            <li>
              <span class="action">{label(action)}</span>
              {#if unavailable}
                {#if binding.command}
                  <code>{binding.command}</code>
                  <button class="btn icon ghost small" aria-label={t('settings.hotkeys.copy')} title={t('settings.hotkeys.copy')} onclick={() => copy(binding.command)}>
                    <Copy size={13} strokeWidth={1.5} />
                  </button>
                {:else}
                  <span class="hint">{t('settings.hotkeys.holdOnly')}</span>
                {/if}
              {:else}
                <button
                  class="key"
                  class:recording={recording === action}
                  class:empty={!binding.shortcut}
                  aria-label={t('settings.hotkeys.record', { action: fullLabel(action) })}
                  aria-describedby={problem ? `hk-${action}` : undefined}
                  onclick={() => (recording === action ? stop() : start(action))}
                  onkeydown={(event) => onkeydown(action, event)}
                  onkeyup={(event) => onkeyup(action, event)}
                  onblur={() => recording === action && stop()}
                >
                  {#if recording === action}
                    {preview || t('settings.hotkeys.press')}
                  {:else}
                    {binding.shortcut ? displayShortcut(binding.shortcut) : t('settings.hotkeys.notSet')}
                  {/if}
                </button>
                <button
                  class="btn icon ghost small"
                  aria-label={t('settings.hotkeys.clear', { action: fullLabel(action) })}
                  title={t('settings.hotkeys.clear', { action: fullLabel(action) })}
                  disabled={!binding.shortcut}
                  onclick={() => save(action, null)}
                >
                  <X size={13} strokeWidth={1.5} />
                </button>
              {/if}
              {#if problem}
                <p class="problem" id="hk-{action}">{problem}</p>
              {:else if recording === action}
                <p class="note">{t('settings.hotkeys.recordHint')}</p>
              {:else if binding.system_trigger && binding.system_trigger !== binding.shortcut}
                <p class="note">{t('settings.hotkeys.assigned', { trigger: binding.system_trigger })}</p>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/each}
  </div>
{/if}

<style>
  .hotkeys {
    display: grid;
    gap: 16px;
    padding: 14px 24px;
  }

  .status {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
  }

  .badge.off {
    color: var(--app-amber-ink);
  }

  .group {
    display: grid;
    gap: 6px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 4px;
  }

  li {
    display: grid;
    grid-template-columns: 1fr auto auto;
    align-items: center;
    column-gap: 8px;
    padding: 4px 8px 4px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .name,
  .action {
    color: var(--ink);
  }

  .hint {
    font-size: 13px;
    color: var(--muted);
  }

  .key {
    min-width: 168px;
    padding: 4px 10px;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--ink);
    text-align: center;
    background: var(--bg-raised);
    border: 1px solid var(--line-strong);
    cursor: pointer;
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

  code {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--ink);
  }

  .problem,
  .note {
    grid-column: 1 / -1;
    font-size: 12px;
  }

  .problem {
    color: var(--danger);
  }

  .note {
    color: var(--muted);
  }
</style>
