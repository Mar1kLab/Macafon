<script lang="ts">
  // DISP-010: edit a profile without recapturing it — per monitor on/off, main, mode, rotation
  // and HDR; the name, the icon and the sound output (DISP-009). Positions stay as captured;
  // a monitor turned on goes right of the others.
  import Dialog from '../components/Dialog.svelte';
  import Dropdown from '../components/Dropdown.svelte';
  import Toggle from '../components/Toggle.svelte';
  import IconPicker from './IconPicker.svelte';
  import LayoutSketch from './LayoutSketch.svelte';
  import type { DisplayMode, DisplayProfile, DisplayRotation } from '../ipc/protocol';
  import { daemon } from '../state';
  import { t, type MessageKey } from '../i18n';
  import { ROTATIONS, connectedOf, makePrimary, modeLabel, profileSketch, sameMode, setLayout, turnOff, turnOn } from './profiles';
  import Field from '../components/Field.svelte';
  import Input from '../components/Input.svelte';

  let { profile, onclose }: { profile: DisplayProfile | null; onclose: () => void } = $props();

  let draft = $state<DisplayProfile | null>(null);
  let busy = $state(false);

  $effect(() => {
    draft = profile ? structuredClone($state.snapshot(profile) as DisplayProfile) : null;
  });

  const monitors = $derived(daemon.displays?.monitors ?? []);
  const outputs = $derived([
    { value: '', label: t('displays.outputKeep') },
    ...(daemon.mixer?.outputs ?? []).map((output) => ({ value: output.name, label: output.description })),
  ]);
  const anyOn = $derived(draft?.monitors.some((monitor) => monitor.layout) ?? false);

  /** Modes the monitor has now, with the saved one first if it is not among them. */
  function modesOf(key: string, saved: DisplayMode | null): DisplayMode[] {
    const monitor = draft?.monitors.find((m) => m.identity.path === key);
    const connected = monitor ? connectedOf(monitor, monitors) : null;
    const modes = connected?.modes ?? [];
    return saved && !modes.some((mode) => sameMode(mode, saved)) ? [saved, ...modes] : modes;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!draft || !draft.name.trim() || !anyOn || busy) return;
    busy = true;
    const ok = await daemon.setDisplays(daemon.client.call('displays.update', { profile: { ...draft, name: draft.name.trim() } }));
    busy = false;
    if (ok) onclose();
  }
</script>

<Dialog open={profile !== null} title={t('displays.editTitle', { name: profile?.name ?? '' })} {onclose} width="640px">
  {#if draft}
    <form id="display-edit" class="form" onsubmit={save}>
      <div class="row">
        <Field label={t('displays.name')} grow>
          <Input bind:value={draft.name} maxlength={40} />
        </Field>
        <div class="field">
          <span class="hud-label">{t('displays.icon')}</span>
          <IconPicker value={draft.icon} onchange={(icon) => draft && (draft.icon = icon)} />
        </div>
      </div>

      <LayoutSketch monitors={profileSketch(draft, monitors)} height={88} />

      <ul class="monitors" aria-label={t('displays.monitor')}>
        {#each draft.monitors as monitor (monitor.identity.path)}
          {@const key = monitor.identity.path}
          {@const connected = connectedOf(monitor, monitors)}
          {@const modes = modesOf(key, monitor.layout?.mode ?? null)}
          <li class:off={!monitor.layout}>
            <div class="head">
              <Toggle
                checked={monitor.layout !== null}
                label={`${monitor.name}: ${t('displays.on')}`}
                disabled={!connected && !monitor.layout}
                onchange={(on) => {
                  if (!draft) return;
                  draft.monitors = on ? turnOn(draft.monitors, key, modes[0] ?? { width: 1920, height: 1080, refresh_mhz: 60_000 }) : turnOff(draft.monitors, key);
                }}
              />
              <span class="name">{monitor.name}</span>
              {#if !connected}<span class="missing">{t('displays.notConnected')}</span>{/if}
              {#if monitor.layout}
                <label class="primary">
                  <input type="radio" name="primary" checked={monitor.layout.primary} onchange={() => draft && (draft.monitors = makePrimary(draft.monitors, key))} />
                  {t('displays.primary')}
                </label>
              {/if}
            </div>
            {#if monitor.layout}
              {@const layout = monitor.layout}
              <div class="settings">
                <Dropdown
                  label={t('displays.mode')}
                  width="230px"
                  options={modes.map((mode) => ({ value: modeLabel(mode), label: modeLabel(mode) }))}
                  value={modeLabel(layout.mode)}
                  onchange={(value) => {
                    const mode = modes.find((m) => modeLabel(m) === value);
                    if (draft && mode) draft.monitors = setLayout(draft.monitors, key, { mode });
                  }}
                />
                <Dropdown
                  label={t('displays.rotation')}
                  width="190px"
                  options={ROTATIONS.map((rotation) => ({ value: rotation, label: t(`displays.rotations.${rotation}` as MessageKey) }))}
                  value={layout.rotation}
                  onchange={(rotation: DisplayRotation) => draft && (draft.monitors = setLayout(draft.monitors, key, { rotation }))}
                />
                {#if connected?.hdr_supported}
                  <span class="hdr">
                    <span class="hud-label">{t('displays.hdr')}</span>
                    <Toggle checked={layout.hdr} label={`${monitor.name}: ${t('displays.hdr')}`} onchange={(hdr) => draft && (draft.monitors = setLayout(draft.monitors, key, { hdr }))} />
                  </span>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>

      <div class="field">
        <span class="hud-label">{t('displays.output')}</span>
        <Dropdown
          label={t('displays.output')}
          width="320px"
          options={outputs}
          value={draft.output_device ?? ''}
          onchange={(value) => draft && (draft.output_device = value || undefined)}
        />
        <span class="hint">{t('displays.outputHint')}</span>
      </div>
    </form>
  {/if}
  {#snippet actions()}
    <button class="btn" type="button" onclick={onclose}>{t('common.cancel')}</button>
    <button class="btn invert" type="submit" form="display-edit" disabled={!draft?.name.trim() || !anyOn || busy}>{t('displays.done')}</button>
  {/snippet}
</Dialog>

<style>
  .form { display: flex; flex-direction: column; gap: var(--space-4); }
  .row { display: flex; gap: var(--space-4); align-items: flex-end; flex-wrap: wrap; }
  .field { display: flex; flex-direction: column; gap: var(--space-2); }
  .row > :global(.grow) { min-width: 200px; }
  .monitors { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; border-top: 1px solid var(--line); }
  li { display: flex; flex-direction: column; gap: var(--space-2); padding: 12px 0; border-bottom: 1px solid var(--line); }
  .head { display: flex; align-items: center; gap: var(--space-3); }
  .name { font-size: var(--text-md); color: var(--ink-strong); }
  li.off .name { color: var(--muted); }
  .missing { font-family: var(--font-mono); font-size: var(--text-2xs); color: var(--app-amber-ink); }
  .primary { margin-left: auto; display: inline-flex; align-items: center; gap: var(--space-2); font-family: var(--font-mono); font-size: var(--text-xs); text-transform: uppercase; letter-spacing: 0.12em; color: var(--muted); }
  .primary input { accent-color: var(--accent); }
  .settings { display: flex; flex-wrap: wrap; gap: var(--space-2) var(--space-3); align-items: center; padding-left: 52px; }
  .hdr { display: inline-flex; align-items: center; gap: var(--space-2); }
  .hint { font-size: var(--text-sm); color: var(--muted); }
</style>
