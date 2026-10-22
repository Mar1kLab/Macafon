<script lang="ts">
  // Settings → Hotkeys (HK-002, HK-004): every action with its shortcut, recorded by pressing
  // the keys; conflicts inside Macafon are caught before the daemon is asked. Without a global
  // shortcut service the CLI commands are shown for the desktop keyboard settings instead.
  import { X } from '@lucide/svelte';
  import Copy from '../icons/animated/Copy.svelte';
  import { daemon } from '../state';
  import { t, type MessageKey } from '../i18n';
  import { DaemonError } from '../ipc/client';
  import type { HotkeyBinding } from '../ipc/protocol';
  import { displayShortcut, modifierChord, recordShortcut } from './shortcut';
  import { tip } from '../components/tooltip.svelte';

  const hotkeys = $derived(daemon.hotkeys);
  const unavailable = $derived(hotkeys?.backend === 'unavailable');

  /** Action being recorded and the modifiers held so far. */
  let recording = $state<string | null>(null);
  let preview = $state('');
  const sides = new Set<string>();
  /** Modifiers held with no key yet that form a valid chord on release. */
  let chord: string | null = null;
  /** Inline problem per action: bad key, conflict, daemon refusal. */
  let problems = $state<Record<string, string>>({});

  function label(action: string): string {
    if (action.startsWith('display_profile.')) {
      const id = action.slice('display_profile.'.length);
      const name = daemon.displays?.profiles.find((entry) => entry.profile.id === id)?.profile.name ?? id;
      return t('settings.hotkeys.displayProfile', { name });
    }
    const kind = action.split('.')[0]!;
    return t(`settings.hotkeys.actions.${kind}` as MessageKey);
  }

  function fullLabel(action: string): string {
    if (action.startsWith('display_profile.')) return label(action);
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
      // Single sounds get their shortcuts on the Soundpad screen.
      if (binding.action.startsWith('soundpad_play.')) continue;
      const soundpad = binding.action.startsWith('soundpad_');
      const dictation = binding.action.startsWith('dictation_');
      const displays = binding.action.startsWith('display_');
      const channel = soundpad || dictation || displays ? undefined : binding.action.split('.')[1];
      const general = binding.action === 'quick_panel' || binding.action.startsWith('master_');
      const id = channel ?? (general ? 'general' : soundpad ? 'soundpad' : dictation ? 'dictation' : displays ? 'displays' : '');
      let group = result.find((g) => g.id === id);
      if (!group) {
        const title = channel
          ? t('settings.hotkeys.channel', { name: channelName(channel) })
          : general
            ? t('settings.hotkeys.general')
            : soundpad
              ? t('settings.hotkeys.soundpad')
              : dictation
                ? t('settings.hotkeys.dictation')
                : displays
                  ? t('settings.hotkeys.displays')
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
    sides.clear();
    recording = action;
    preview = '';
    setProblem(action, null);
  }

  function stop() {
    sides.clear();
    chord = null;
    recording = null;
    preview = '';
  }

  async function save(action: string, shortcut: string | null) {
    try {
      const [id, slot] = action.split('#');
      const next = await daemon.client.call('hotkeys.set', { action: id!, shortcut, secondary: slot === 'secondary' });
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
    if (/^(Control|Alt)(Left|Right)$/.test(event.code)) sides.add(event.code);
    const result = recordShortcut(event, daemon.windows ? sides : undefined);
    if (result.kind === 'partial') {
      preview = result.preview;
      chord = daemon.windows ? modifierChord(event) : null;
      return;
    }
    chord = null;
    if (result.kind === 'needs_modifier') {
      setProblem(action, t('settings.hotkeys.needsModifier', { shortcut: result.preview }));
      return;
    }
    if (result.kind === 'unsupported') {
      setProblem(action, t('settings.hotkeys.unsupported', { shortcut: result.preview }));
      return;
    }
    finish(action, result.shortcut);
  }

  function finish(action: string, shortcut: string) {
    stop();
    const taken = hotkeys?.bindings.find((b) => (b.action !== action && b.shortcut === shortcut) || (`${b.action}#secondary` !== action && b.secondary_shortcut === shortcut));
    if (taken) {
      setProblem(action, t('settings.hotkeys.conflict', { shortcut: displayShortcut(shortcut), action: fullLabel(taken.action) }));
      return;
    }
    void save(action, shortcut);
  }

  function onkeyup(action: string, event: KeyboardEvent) {
    if (recording !== action) return;
    event.preventDefault();
    if (chord) {
      const next = chord;
      chord = null;
      return finish(action, next);
    }
    sides.delete(event.code);
    const result = recordShortcut(event, daemon.windows ? sides : undefined);
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
    {#each groups as group (group.id)}
      <div class="group">
        <h3 class="hud-label">{group.title}</h3>
        <ul>
          {#each group.bindings as binding (binding.action)}
            {@const action = binding.action}
            {@const problem = problems[action] ?? binding.error}
            <li class:unavailable>
              <span class="action">{label(action)}</span>
              {#if unavailable}
                {#if binding.command}
                  <code>{binding.command}</code>
                  <button class="btn icon ghost small" aria-label={t('settings.hotkeys.copy')} {@attach tip(t('settings.hotkeys.copy'))} onclick={() => copy(binding.command)}>
                    <Copy size={13} strokeWidth={1.5} />
                  </button>
                {:else}
                  <span class="hint">{t('settings.hotkeys.holdOnly')}</span>
                {/if}
              {:else}
                {#each [false, true] as secondary}
                  {@const slot = secondary ? `${action}#secondary` : action}
                  {@const shortcut = secondary ? binding.secondary_shortcut : binding.shortcut}
                  <div class="slot">
                    <button class="key" class:recording={recording === slot} class:empty={!shortcut}
                      aria-label={`${t('settings.hotkeys.record', { action: fullLabel(action) })} / ${t(secondary ? 'settings.hotkeys.secondary' : 'settings.hotkeys.primary')}`}
                      aria-describedby={(secondary ? problems[slot] || binding.secondary_error : problem) ? `hk-${slot}` : undefined}
                      onclick={() => recording === slot ? stop() : start(slot)}
                      onkeydown={(event) => onkeydown(slot, event)} onkeyup={(event) => onkeyup(slot, event)}
                      onblur={() => recording === slot && stop()}>
                      {recording === slot ? preview || t('settings.hotkeys.press') : shortcut ? displayShortcut(shortcut) : t(secondary ? 'settings.hotkeys.secondary' : 'settings.hotkeys.notSet')}
                    </button>
                    <button class="btn icon ghost small" aria-label={`${t('settings.hotkeys.clear', { action: fullLabel(action) })} / ${t(secondary ? 'settings.hotkeys.secondary' : 'settings.hotkeys.primary')}`}
                      disabled={!shortcut} onclick={() => save(slot, null)}><X size={13} strokeWidth={1.5} /></button>
                  </div>
                  {#if secondary && (problems[slot] || binding.secondary_error)}<p class="problem" id="hk-{slot}">{problems[slot] ?? binding.secondary_error}</p>{/if}
                {/each}
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
    gap: var(--space-4);
    padding: 14px 24px;
  }

  .slot { display: flex; min-width: 0; align-items: center; }

  .group {
    display: grid;
    gap: var(--space-2);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: var(--space-1);
  }

  li {
    display: grid;
    grid-template-columns: minmax(100px, 1fr) minmax(0, 1fr) minmax(0, 1fr);
    align-items: center;
    column-gap: var(--space-2);
    padding: 4px 8px 4px 12px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  li.unavailable { grid-template-columns: minmax(100px, 1fr) minmax(0, 2fr) auto; }

  .action {
    color: var(--ink);
  }

  .hint {
    font-size: var(--text-md);
    color: var(--muted);
  }

  .key {
    min-width: 0;
    width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    padding: 4px 10px;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
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
    overflow-wrap: anywhere;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--ink);
  }

  .problem,
  .note {
    grid-column: 1 / -1;
    font-size: var(--text-sm);
  }

  .problem {
    color: var(--danger);
  }

  .note {
    color: var(--muted);
  }
</style>
