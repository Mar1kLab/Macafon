<script lang="ts">
  // 03 / SETTINGS (PRD §9.5) — what the daemon supports today: start with system (AS-001),
  // the setup guide (§9.2), fallback microphone (MIC-006), input channel (MIC-004), system
  // default microphone (MIC-012), virtual microphone format (MIC-011), quit (BG-005), about.
  // Mixer: known-app sorting (APP-006) and the output priority list (OUT-002). The soundpad
  // library folder (SND-001). Hotkeys (HK-002, HK-004) and their popup (HK-006, where the
  // daemon draws one), shown in pictures with the overlay (HK-007, SND-006). Doctor (§9.6).
  // A section list on the left follows the scroll and jumps to a section.
  import { onMount, tick } from 'svelte';
  import { availableMonitors, primaryMonitor } from '@tauri-apps/api/window';
  import { isTauri } from '../lib/ipc/tauri';
  import Toggle from '../lib/components/Toggle.svelte';
  import Dropdown from '../lib/components/Dropdown.svelte';
  import Segmented from '../lib/components/Segmented.svelte';
  import RulerSlider from '../lib/components/RulerSlider.svelte';
  import Dialog from '../lib/components/Dialog.svelte';
  import OutputDevicePanel from '../lib/mixer/OutputDevicePanel.svelte';
  import { ArrowDown, ArrowLeft, ArrowUp, X } from '@lucide/svelte';
  import { daemon } from '../lib/state';
  import { nav, type SettingsSection } from '../lib/state/nav.svelte';
  import Doctor from '../lib/settings/Doctor.svelte';
  import Hotkeys from '../lib/settings/Hotkeys.svelte';
  import MonitorPicture from '../lib/settings/MonitorPicture.svelte';
  import OverlayLayoutPicture from '../lib/settings/OverlayLayoutPicture.svelte';
  import OverlayBuilder from '../lib/settings/OverlayBuilder.svelte';
  import { DEFAULT_OVERLAY } from '../lib/quick/overlay';
  import PictureChoice from '../lib/settings/PictureChoice.svelte';
  import PopupPicture from '../lib/settings/PopupPicture.svelte';
  import LookPicture from '../lib/settings/LookPicture.svelte';
  import LibraryFolder from '../lib/soundpad/LibraryFolder.svelte';
  import PopupLook from '../lib/dictation/PopupLook.svelte';
  import { t } from '../lib/i18n';
  import type {
    CloseAction,
    InputChannel,
    OsdEdge,
    OsdLook,
    OsdPrefs,
    OsdStyle,
    OverlayExpand,
    OverlayPlacement,
    OverlayPrefs,
    SoundpadOverlayControls,
    SoundpadOverlayView,
  } from '../lib/ipc/protocol';
  import { OSD_MAX_OFFSET_PCT, QUANTUM_CHOICES, SAMPLE_RATE_HZ } from '../lib/ipc/dsp';

  let { onnavigate }: { onnavigate: (screen: 'microphone' | 'mixer') => void } = $props();
  const mic = $derived(daemon.mic);
  const mixer = $derived(daemon.mixer);
  let outputPanel = $state<string | null>(null);
  const describe = (name: string) => mixer?.outputs.find((output) => output.name === name)?.description ?? name;
  const unlisted = $derived(
    (mixer?.outputs ?? [])
      .filter((output) => !mixer!.output_priority.includes(output.name))
      .map((output) => ({ value: output.name, label: output.description })),
  );

  function setPriority(devices: string[]) {
    void daemon.setMixer(daemon.client.call('mixer.set_output_priority', { devices }));
  }

  function shift(index: number, delta: number) {
    if (!mixer) return;
    const list = [...mixer.output_priority];
    const [item] = list.splice(index, 1);
    list.splice(index + delta, 0, item!);
    setPriority(list);
  }
  const autostart = $derived(daemon.autostart);
  const fallbackOptions = $derived([
    { value: null as string | null, label: t('settings.none') },
    ...daemon.devices
      .filter((device) => device.name !== mic?.device)
      .map((device) => ({ value: device.name as string | null, label: device.description })),
  ]);
  const CHANNELS: InputChannel[] = ['Auto', 'Left', 'Right', 'Mix'];
  const channelOptions = CHANNELS.map((value) => ({ value, label: t(`settings.channels.${value}`) }));
  const prefs = $derived(daemon.prefs);
  const quantumOptions = QUANTUM_CHOICES.map((frames) => ({
    value: frames,
    label: t('settings.quantumValue', { frames, ms: ((frames / SAMPLE_RATE_HZ) * 1000).toFixed(1) }),
  }));
  const closeOptions = (['tray', 'quit'] as CloseAction[]).map((value) => ({
    value,
    label: t(`settings.closeActions.${value}`),
  }));

  async function setPrefs(patch: {
    volume_step_pct?: number;
    quantum_frames?: number;
    close_action?: CloseAction;
    osd?: OsdPrefs;
    overlay?: OverlayPrefs;
    main_monitor?: string;
  }) {
    const next = await daemon.run(
      daemon.client.call('prefs.set', {
        volume_step_pct: patch.volume_step_pct ?? null,
        quantum_frames: patch.quantum_frames ?? null,
        close_action: patch.close_action ?? null,
        osd: patch.osd ?? null,
        overlay: patch.overlay ?? null,
        ...(patch.main_monitor !== undefined ? { main_monitor: patch.main_monitor } : {}),
      }),
    );
    if (next) daemon.prefs = next;
  }

  // The monitor the main window opens on: the system's monitors, or two made-up ones in a
  // browser (mock daemon). A saved monitor that is unplugged stays listed until changed.
  interface Display {
    name: string;
    label: string;
  }
  let displays = $state<Display[]>([]);
  const displayName = (name: string, index: number) => {
    const number = /(\d+)\s*$/.exec(name)?.[1];
    return t('settings.monitorName', { n: number ?? String(index + 1) });
  };
  onMount(() => {
    if (!isTauri()) {
      displays = [
        { name: 'mock-display-1', label: `${displayName('DISPLAY1', 0)} · 2560×1440 · ${t('settings.monitorPrimary')}` },
        { name: 'mock-display-2', label: `${displayName('DISPLAY2', 1)} · 1920×1080` },
      ];
      return;
    }
    void Promise.all([availableMonitors(), primaryMonitor()])
      .then(([monitors, primary]) => {
        displays = monitors
          .filter((monitor) => monitor.name)
          .map((monitor, index) => ({
            name: monitor.name!,
            label: [
              displayName(monitor.name!, index),
              `${monitor.size.width}×${monitor.size.height}`,
              ...(monitor.name === primary?.name ? [t('settings.monitorPrimary')] : []),
            ].join(' · '),
          }));
      })
      .catch(() => (displays = []));
  });
  const monitorChoices = $derived([
    { value: '', label: t('settings.monitorAuto') },
    ...displays.map((display) => ({ value: display.name, label: display.label })),
    ...(prefs?.main_monitor && !displays.some((display) => display.name === prefs.main_monitor)
      ? [{ value: prefs.main_monitor, label: t('settings.monitorMissing', { name: displayName(prefs.main_monitor, 0) }) }]
      : []),
  ]);

  // HK-006: each style has its own pair of edges; the daemon shows a sample popup on every change.
  const OSD_EDGES: Record<OsdStyle, OsdEdge[]> = { vertical: ['left', 'right'], horizontal: ['top', 'bottom'] };
  const osdStyleOptions = (['vertical', 'horizontal'] as OsdStyle[]).map((value) => ({
    value,
    label: t(`settings.osd.styles.${value}`),
    hint: t(`settings.osd.styleHints.${value}`),
  }));
  const osdEdgeOptions = $derived(
    OSD_EDGES[prefs?.osd.style ?? 'vertical'].map((value) => ({ value, label: t(`settings.osd.edges.${value}`) })),
  );
  const lookOptions = (['classic', 'sketch'] as OsdLook[]).map((value) => ({
    value,
    label: t(`settings.osd.looks.${value}`),
    hint: t(`settings.osd.lookHints.${value}`),
  }));
  const overlayViewOptions = (['grid', 'list'] as SoundpadOverlayView[]).map((value) => ({ value, label: t(`settings.overlay.views.${value}`) }));
  const overlayControlsOptions = (['top', 'bottom', 'hidden'] as SoundpadOverlayControls[]).map((value) => ({
    value,
    label: t(`settings.overlay.controlsPlaces.${value}`),
  }));
  const monitorOptions = [false, true].map((value) => ({
    value,
    label: t(value ? 'settings.osd.primaryMonitor' : 'settings.osd.activeMonitor'),
    hint: t(value ? 'settings.osd.primaryHint' : 'settings.osd.activeHint'),
  }));
  // §9.9: the hotkey overlay's parts, where its side panels open and how it is placed.
  const overlay = $derived(prefs?.overlay ?? DEFAULT_OVERLAY);
  const expandOptions = (['right', 'left', 'down'] as OverlayExpand[]).map((value) => ({
    value,
    label: t(`settings.overlay.expands.${value}`),
    hint: t(`settings.overlay.expandHints.${value}`),
  }));
  const placementOptions = (['anchored', 'centered'] as OverlayPlacement[]).map((value) => ({
    value,
    label: t(`settings.overlay.placements.${value}`),
    hint: t(`settings.overlay.placementHints.${value}`),
  }));

  function setOverlay(patch: Partial<OverlayPrefs>) {
    void setPrefs({ overlay: { ...overlay, ...patch } });
  }

  function setOsd(patch: Partial<OsdPrefs>) {
    if (!prefs) return;
    const osd = { ...prefs.osd, ...patch };
    const edges = OSD_EDGES[osd.style];
    if (!edges.includes(osd.edge)) osd.edge = edges[1]!;
    void setPrefs({ osd });
  }

  // Dictation popup (STT-016, §9.9.3): look, position and the overlay tile.
  const dictationLooks = [
    { value: 'flow' as const, label: t('dictation.popup.flow'), hint: t('dictation.popup.flowHint') },
    { value: 'analog' as const, label: t('dictation.popup.analog'), hint: t('dictation.popup.analogHint') },
  ];
  const dictationPositions = [
    { value: 'bottom' as const, label: t('dictation.popup.bottom') },
    { value: 'top' as const, label: t('dictation.popup.top') },
  ];

  // The section list: what this platform and daemon show, in page order.
  type SectionId = SettingsSection;
  const sections = $derived(
    (
      [
        ['general', t('settings.general'), true],
        ['audio', t('settings.audio'), !!mic],
        ['outputs', t('settings.outputs'), daemon.outputControls.length > 0],
        ['mixer', t('settings.mixer'), !!mixer],
        ['soundpad', t('settings.soundpad'), !!daemon.soundpad],
        ['hotkeys', t('settings.hotkeys.title'), !!daemon.hotkeys],
        ['popup', t('settings.osd.title'), !!prefs && !!daemon.hello?.capabilities.hotkey_popup],
        ['overlay', t('settings.overlay.title'), !!prefs],
        ['dictation', t('settings.dictationPopup.title'), !!daemon.dictation],
        ['doctor', t('settings.doctor'), true],
        ['about', t('settings.about'), true],
      ] as [SectionId, string, boolean][]
    )
      .filter(([, , shown]) => shown)
      .map(([id, label]) => ({ id, label })),
  );
  let scroller: HTMLDivElement | undefined = $state();
  let active = $state<SectionId>('general');
  let spyFrame = 0;

  /** The section whose top passed the top of the list, or the last one at the very bottom. */
  function spy() {
    cancelAnimationFrame(spyFrame);
    spyFrame = requestAnimationFrame(() => {
      if (!scroller) return;
      const top = scroller.getBoundingClientRect().top + 48;
      let current = sections[0]?.id ?? 'general';
      for (const { id } of sections) {
        const element = document.getElementById(`settings-${id}`);
        if (element && element.getBoundingClientRect().top <= top) current = id;
      }
      const bottom = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2;
      active = bottom && scroller.scrollTop > 0 ? (sections.at(-1)?.id ?? current) : current;
    });
  }

  async function jump(id: SectionId, smooth = true) {
    await tick();
    const element = document.getElementById(`settings-${id}`);
    if (!element) return;
    const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
    element.scrollIntoView({ behavior: smooth && !reduce ? 'smooth' : 'auto', block: 'start' });
    element.querySelector<HTMLElement>('h2')?.focus({ preventScroll: true });
    active = id;
  }

  // A screen's gear (or `?section=popup`) opens Settings at that section once the daemon has
  // shown it, and keeps it in place while sections above still fill in, until the user scrolls
  // or 3 s pass.
  let pinFrame = 0;
  let handled = -1;
  $effect(() => {
    const request = nav.section;
    if (!request || request.seq === handled) return;
    const section = sections.find(({ id }) => id === request.id);
    if (!section || !daemon.connected) return;
    handled = request.seq;
    void jump(section.id, false).then(() => pin(section.id));
  });

  function pin(id: SectionId) {
    const until = performance.now() + 3000;
    const stop = () => {
      cancelAnimationFrame(pinFrame);
      for (const type of ['wheel', 'pointerdown', 'keydown'] as const) scroller?.removeEventListener(type, stop);
    };
    for (const type of ['wheel', 'pointerdown', 'keydown'] as const) scroller?.addEventListener(type, stop, { once: true });
    let top = document.getElementById(`settings-${id}`)?.offsetTop;
    const follow = () => {
      const element = document.getElementById(`settings-${id}`);
      if (!element || performance.now() > until) return stop();
      if (element.offsetTop !== top) {
        top = element.offsetTop;
        element.scrollIntoView({ block: 'start' });
        active = id;
      }
      pinFrame = requestAnimationFrame(follow);
    };
    pinFrame = requestAnimationFrame(follow);
  }

  onMount(() => () => {
    cancelAnimationFrame(spyFrame);
    cancelAnimationFrame(pinFrame);
  });

  let confirmQuit = $state(false);
  let confirmDefault = $state(false);
  const version = $derived(daemon.status.state === 'connected' ? daemon.status.version : '—');

  async function setAutostart(enabled: boolean) {
    const state = await daemon.run(daemon.client.call('autostart.set', { enabled }));
    if (state) daemon.autostart = state;
  }

  // MIC-012: the switch shows what PipeWire reports; turning it on asks first.
  async function makeDefault() {
    confirmDefault = false;
    await daemon.run(daemon.client.call('mic.set_default_source', { confirmed: true }));
  }

  async function setDefault(on: boolean) {
    if (on) confirmDefault = true;
    else await daemon.run(daemon.client.call('mic.restore_default_source'));
  }
</script>

<div class="screen">
  <header class="top">
    <button class="btn small back" title={t('nav.backTo', { screen: t(`nav.${nav.previous}`) })} onclick={() => nav.back()}>
      <ArrowLeft size={13} strokeWidth={1.5} />{t('nav.back')}
    </button>
    <h1 class="hud-label">{t('settings.section')}</h1>
  </header>
  <div class="body">
    <nav class="sections" aria-label={t('settings.sections')}>
      <ol>
        {#each sections as section, index (section.id)}
          <li>
            <button class:active={active === section.id} aria-current={active === section.id ? 'true' : undefined} onclick={() => jump(section.id)}>
              <span class="n">{String(index + 1).padStart(2, '0')}</span>{section.label}
            </button>
          </li>
        {/each}
      </ol>
    </nav>
    <div class="scroll" bind:this={scroller} onscroll={spy}>
      <section id="settings-general">
        <h2 class="hud-label" tabindex="-1">{t('settings.general')}</h2>
        <div class="row">
          <p class="name">{t('settings.startWithSystem')}</p>
          <Toggle checked={autostart?.enabled ?? false} disabled={!autostart} label={t('settings.startWithSystem')} onchange={setAutostart} />
        </div>
        <div class="row">
          <p class="name">{t('settings.onboarding')}</p>
          <button class="btn" onclick={() => daemon.startOnboarding()}>{t('settings.onboardingRun')}</button>
        </div>
        {#if prefs}
          <div class="row">
            <p class="name">{t('settings.closeAction')}</p>
            <Dropdown
              label={t('settings.closeAction')}
              options={closeOptions}
              value={prefs.close_action}
              width="280px"
              onchange={(close_action) => setPrefs({ close_action })}
            />
          </div>
        {/if}
        {#if prefs}
          <div class="row">
            <div>
              <p class="name">{t('settings.mainMonitor')}</p>
              <p class="hint">{t('settings.mainMonitorHint')}</p>
            </div>
            <Dropdown
              label={t('settings.mainMonitor')}
              options={monitorChoices}
              value={prefs.main_monitor}
              width="280px"
              onchange={(main_monitor) => setPrefs({ main_monitor })}
            />
          </div>
        {/if}
        <div class="row">
          <p class="name">{t('settings.quit')}</p>
          <button class="btn" onclick={() => (confirmQuit = true)}>{t('settings.quit')}</button>
        </div>
      </section>

      {#if mic}
        <section id="settings-audio">
          <h2 class="hud-label" tabindex="-1">{t('settings.audio')}</h2>
          <div class="row">
            <p class="name">{t('settings.fallback')}</p>
            <Dropdown
              label={t('settings.fallback')}
              options={fallbackOptions}
              value={mic.fallback_device}
              width="280px"
              onchange={(device) => daemon.setMic(daemon.client.call('mic.set_fallback_device', { device }))}
            />
          </div>
          <div class="row">
            <p class="name">{t('settings.inputChannel')}</p>
            <Dropdown
              label={t('settings.inputChannel')}
              options={channelOptions}
              value={mic.input_channel ?? 'Auto'}
              width="280px"
              onchange={(channel) => daemon.setMic(daemon.client.call('mic.set_input_channel', { channel }))}
            />
          </div>
          <div class="row">
            <p class="name">{t('settings.systemDefault')}</p>
            <Toggle checked={mic.is_default_source} disabled={daemon.cableMissing} label={t('settings.systemDefault')} onchange={setDefault} />
          </div>
          {#if prefs && (daemon.hello?.capabilities.graph_quantum ?? true)}
            <div class="row">
              <p class="name">{t('settings.quantum')}</p>
              <Dropdown
                label={t('settings.quantum')}
                options={quantumOptions}
                value={prefs.quantum_frames}
                width="280px"
                onchange={(quantum_frames) => setPrefs({ quantum_frames })}
              />
            </div>
          {/if}
          <div class="row">
            <p class="name">{t('settings.format')}</p>
            <span class="badge">{mic.stereo || daemon.windows ? t('settings.stereo') : t('settings.mono')} · 48 kHz · float32</span>
          </div>
        </section>
      {/if}

      {#if daemon.outputControls.length}
        <section id="settings-outputs">
          <h2 class="hud-label" tabindex="-1">{t('settings.outputs')}</h2>
          {#each daemon.outputControls as device (device.id)}
            <div class="row">
              <span class="name">{device.name}</span>
              <button class="btn small" onclick={() => (outputPanel = device.id)}>{t('outputDevice.open')}</button>
            </div>
          {/each}
        </section>
      {/if}

      {#if mixer}
        <section id="settings-mixer">
          <h2 class="hud-label" tabindex="-1">{t('settings.mixer')}</h2>
          {#if daemon.windows}
            <div class="row">
              <span class="name">{t('settings.mixerMode')}</span>
              <div class="mode" role="group" aria-label={t('settings.mixerMode')}>
                <button class="btn on" aria-pressed="true">{t('settings.mixerSessions')}</button>
                <button class="btn" disabled>{t('settings.mixerCable')}</button>
              </div>
            </div>
          {/if}
          <div class="row">
            <p class="name">{t('settings.autoClassify')}</p>
            <Toggle
              checked={mixer.auto_classify}
              label={t('settings.autoClassify')}
              onchange={(enabled) => daemon.setMixer(daemon.client.call('mixer.set_auto_classify', { enabled }))}
            />
          </div>
          <div class="row column">
            <p class="name">{t('settings.outputPriority')}</p>
            <ol class="priority">
              {#each mixer.output_priority as name, index (name)}
                <li class:missing={!mixer.outputs.some((o) => o.name === name)}>
                  <span class="n">{index + 1}</span>
                  <span class="device">{describe(name)}</span>
                  {#if !mixer.outputs.some((o) => o.name === name)}<span class="badge">{t('settings.disconnected')}</span>{/if}
                  <button class="btn icon ghost small" aria-label={t('settings.moveUp')} disabled={index === 0} onclick={() => shift(index, -1)}><ArrowUp size={13} strokeWidth={1.5} /></button>
                  <button class="btn icon ghost small" aria-label={t('settings.moveDown')} disabled={index === mixer.output_priority.length - 1} onclick={() => shift(index, 1)}><ArrowDown size={13} strokeWidth={1.5} /></button>
                  <button class="btn icon ghost small" aria-label={t('settings.remove')} onclick={() => setPriority(mixer.output_priority.filter((n) => n !== name))}><X size={13} strokeWidth={1.5} /></button>
                </li>
              {:else}
                <li class="empty">{t('settings.priorityEmpty')}</li>
              {/each}
            </ol>
            {#if unlisted.length}
              <Dropdown
                label={t('settings.addOutput')}
                options={[{ value: '', label: t('settings.addOutput') }, ...unlisted]}
                value={''}
                width="280px"
                onchange={(name) => name && setPriority([...mixer.output_priority, name])}
              />
            {/if}
          </div>
        </section>
      {/if}

      {#if daemon.soundpad}
        <section id="settings-soundpad">
          <h2 class="hud-label" tabindex="-1">{t('settings.soundpad')}</h2>
          <div class="row column">
            <div>
              <p class="name">{t('soundpad.folder')}</p>
              <p class="hint">{t('soundpad.folderHint')}</p>
            </div>
            <LibraryFolder />
          </div>
        </section>
      {/if}

      {#if daemon.hotkeys}
        <section id="settings-hotkeys">
          <h2 class="hud-label" tabindex="-1">{t('settings.hotkeys.title')}</h2>
          {#if prefs}
            <div class="row">
              <span class="name">{t('settings.hotkeys.step')}</span>
              <div class="slider">
                <RulerSlider compact label={t('settings.hotkeys.step')} value={prefs.volume_step_pct} min={1} max={20} step={1} defaultValue={5} format={(value) => `${value}%`} onchange={(volume_step_pct) => setPrefs({ volume_step_pct })} />
              </div>
            </div>
          {/if}
          <Hotkeys />
        </section>
      {/if}

      {#if prefs && daemon.hello?.capabilities.hotkey_popup}
        <section id="settings-popup">
          <h2 class="hud-label" tabindex="-1">{t('settings.osd.title')}</h2>
          <div class="row column">
            <div>
              <p class="name">{t('settings.osd.look')}</p>
              <p class="hint">{t('settings.osd.lookHint')}</p>
            </div>
            <PictureChoice label={t('settings.osd.look')} options={lookOptions} value={prefs.osd.look} onchange={(look) => setOsd({ look })}>
              {#snippet picture(look)}<LookPicture {look} />{/snippet}
            </PictureChoice>
          </div>
          <div class="row column">
            <p class="name">{t('settings.osd.style')}</p>
            <PictureChoice label={t('settings.osd.style')} options={osdStyleOptions} value={prefs.osd.style} onchange={(style) => setOsd({ style })}>
              {#snippet picture(style)}
                <PopupPicture {style} edge={style === prefs.osd.style ? prefs.osd.edge : OSD_EDGES[style][1]!} />
              {/snippet}
            </PictureChoice>
          </div>
          <div class="split">
            <div class="controls">
              <div class="row">
                <p class="name">{t('settings.osd.edge')}</p>
                <Segmented label={t('settings.osd.edge')} layout="inline" options={osdEdgeOptions} value={prefs.osd.edge} onchange={(edge) => setOsd({ edge })} />
              </div>
              <div class="row">
                <p class="name">{t('settings.osd.offset')}</p>
                <div class="slider">
                  <RulerSlider
                    compact
                    label={t('settings.osd.offset')}
                    value={prefs.osd.offset_pct}
                    min={0}
                    max={OSD_MAX_OFFSET_PCT}
                    defaultValue={0}
                    step={0.5}
                    format={(value) => `${value}%`}
                    onchange={(offset_pct) => setOsd({ offset_pct })}
                  />
                </div>
              </div>
              <div class="row">
                <p class="name">{t('settings.osd.preview')}</p>
                <button class="btn" onclick={() => setOsd({})}>{t('settings.osd.previewRun')}</button>
              </div>
            </div>
            <figure class="live">
              <PopupPicture style={prefs.osd.style} edge={prefs.osd.edge} offsetPct={prefs.osd.offset_pct} />
              <figcaption class="hud-label">{t('settings.osd.where')}</figcaption>
            </figure>
          </div>
          <div class="row column">
            <p class="name">{t('settings.osd.monitor')}</p>
            <PictureChoice label={t('settings.osd.monitor')} options={monitorOptions} value={prefs.osd.primary_monitor} onchange={(primary_monitor) => setOsd({ primary_monitor })}>
              {#snippet picture(primary)}<MonitorPicture {primary} />{/snippet}
            </PictureChoice>
          </div>
        </section>
      {/if}

      {#if prefs}
        <section id="settings-overlay">
          <h2 class="hud-label" tabindex="-1">{t('settings.overlay.title')}</h2>
          <div class="row">
            <div>
              <p class="name">{t('settings.overlay.enabled')}</p>
              <p class="hint">{t('settings.overlay.enabledHint')}</p>
            </div>
            <Toggle checked={overlay.enabled} label={t('settings.overlay.enabled')} onchange={(enabled) => setOverlay({ enabled })} />
          </div>
          <div class="overlay-setup" class:off={!overlay.enabled} inert={!overlay.enabled}>
          <div class="row column">
            <div>
              <p class="name">{t('settings.overlay.parts')}</p>
              <p class="hint">{t('settings.overlay.partsHint')}</p>
            </div>
            <OverlayBuilder setup={overlay} onchange={setOverlay} />
          </div>
          <div class="row column">
            <div>
              <p class="name">{t('settings.overlay.expand')}</p>
              <p class="hint">{t('settings.overlay.expandHint')}</p>
            </div>
            <PictureChoice label={t('settings.overlay.expand')} options={expandOptions} value={overlay.expand} onchange={(expand) => setOverlay({ expand })}>
              {#snippet picture(expand)}<OverlayLayoutPicture {expand} placement={overlay.placement} />{/snippet}
            </PictureChoice>
          </div>
          <div class="row column">
            <div>
              <p class="name">{t('settings.overlay.placement')}</p>
              <p class="hint">{overlay.expand === 'down' ? t('settings.overlay.placementDown') : t('settings.overlay.placementHint')}</p>
            </div>
            <PictureChoice label={t('settings.overlay.placement')} options={placementOptions} value={overlay.placement} onchange={(placement) => setOverlay({ placement })}>
              {#snippet picture(placement)}<OverlayLayoutPicture expand={overlay.expand === 'down' ? 'right' : overlay.expand} {placement} />{/snippet}
            </PictureChoice>
          </div>
          {#if daemon.soundpad?.prefs.show_in_overlay}
            <div class="row">
              <p class="name">{t('settings.overlay.view')}</p>
              <Segmented
                label={t('settings.overlay.view')}
                layout="inline"
                options={overlayViewOptions}
                value={daemon.soundpad.prefs.overlay_view}
                onchange={(overlay_view) => daemon.setSoundpadPrefs({ overlay_view })}
              />
            </div>
            <div class="row">
              <div>
                <p class="name">{t('settings.overlay.controls')}</p>
                <p class="hint">{t('settings.overlay.controlsHint')}</p>
              </div>
              <Segmented
                label={t('settings.overlay.controls')}
                layout="inline"
                options={overlayControlsOptions}
                value={daemon.soundpad.prefs.overlay_controls}
                onchange={(overlay_controls) => daemon.setSoundpadPrefs({ overlay_controls })}
              />
            </div>
          {/if}
          </div>
        </section>
      {/if}

      {#if daemon.dictation}
        {@const dictation = daemon.dictation.prefs}
        <section id="settings-dictation">
          <h2 class="hud-label" tabindex="-1">{t('settings.dictationPopup.title')}</h2>
          <div class="row column">
            <div>
              <p class="name">{t('dictation.popup.look')}</p>
              <p class="hint">{t('settings.dictationPopup.lookHint')}</p>
            </div>
            <PictureChoice label={t('dictation.popup.look')} options={dictationLooks} value={dictation.popup_look} onchange={(popup_look) => daemon.setDictationPrefs({ popup_look })}>
              {#snippet picture(look)}
                <span class="dictation-look" class:analog={look === 'analog'}><span><PopupLook {look} source={look === dictation.popup_look ? 'demo' : 'still'} /></span></span>
              {/snippet}
            </PictureChoice>
          </div>
          <div class="row column">
            <div>
              <p class="name">{t('settings.osd.monitor')}</p>
              <p class="hint">{t('settings.dictationPopup.monitorHint')}</p>
            </div>
            <PictureChoice label={t('settings.dictationPopup.monitor')} options={monitorOptions} value={dictation.popup_primary_monitor} onchange={(popup_primary_monitor) => daemon.setDictationPrefs({ popup_primary_monitor })}>
              {#snippet picture(primary)}<MonitorPicture {primary} />{/snippet}
            </PictureChoice>
          </div>
          <div class="row">
            <p class="name">{t('dictation.popup.position')}</p>
            <Segmented label={t('dictation.popup.position')} layout="inline" options={dictationPositions} value={dictation.popup_position} onchange={(popup_position) => daemon.setDictationPrefs({ popup_position })} />
          </div>
          <div class="row">
            <p class="name">{t('dictation.popup.offset')}</p>
            <div class="slider">
              <RulerSlider
                compact
                label={t('dictation.popup.offset')}
                value={dictation.popup_offset_pct}
                min={0}
                max={OSD_MAX_OFFSET_PCT}
                defaultValue={0}
                step={0.5}
                format={(value) => `${value}%`}
                onchange={(popup_offset_pct) => daemon.setDictationPrefs({ popup_offset_pct })}
              />
            </div>
          </div>
        </section>
      {/if}

      <section id="settings-doctor">
        <h2 class="hud-label" tabindex="-1">{t('settings.doctor')}</h2>
        <Doctor {onnavigate} onmakedefault={() => (confirmDefault = true)} />
      </section>

      <section id="settings-about">
        <h2 class="hud-label" tabindex="-1">{t('settings.about')}</h2>
        <div class="row">
          <p class="name">{t('settings.version')}</p>
          <span class="value">UI {__APP_VERSION__} · {t('settings.daemonVersion')} {version}</span>
        </div>
        <div class="row">
          <p class="name">{t('settings.licenses')}</p>
          <p class="hint licenses">{t('settings.licenseList')}</p>
        </div>
      </section>
    </div>
  </div>
</div>

<Dialog open={confirmQuit} title={t('settings.quitConfirmTitle')} onclose={() => (confirmQuit = false)}>
  <p class="confirm">{daemon.windows ? t('settings.quitConfirmWindows') : t('settings.quitConfirm')}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (confirmQuit = false)}>{t('common.cancel')}</button>
    <button
      class="btn danger on"
      onclick={() => {
        confirmQuit = false;
        void daemon.run(daemon.client.call('daemon.quit'));
      }}>{t('settings.quit')}</button
    >
  {/snippet}
</Dialog>

<Dialog open={confirmDefault} title={t('settings.makeDefaultTitle')} onclose={() => (confirmDefault = false)}>
  <p class="confirm">{t('settings.makeDefaultConfirm')}</p>
  {#snippet actions()}
    <button class="btn" onclick={() => (confirmDefault = false)}>{t('common.cancel')}</button>
    <button class="btn invert" onclick={makeDefault}>{t('settings.makeDefault')}</button>
  {/snippet}
</Dialog>

{#if outputPanel}
  <OutputDevicePanel id={outputPanel} onclose={() => (outputPanel = null)} />
{/if}

<style>
  .mode { display: flex; flex-wrap: wrap; gap: 4px; }

  .overlay-setup {
    border-top: 1px solid var(--line);
    transition: opacity var(--dur) var(--ease);
  }

  .overlay-setup.off {
    opacity: 0.4;
  }

  .screen {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .top {
    display: flex;
    align-items: center;
    min-height: 59px;
    padding: 14px 24px;
    border-bottom: 1px solid var(--line);
  }

  .top {
    gap: 16px;
  }

  .back {
    gap: 6px;
  }

  h1 {
    color: var(--ink);
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .sections {
    flex: none;
    width: 196px;
    overflow-y: auto;
    padding: 24px 0 24px 24px;
  }

  .sections ol {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
  }

  .sections button {
    display: flex;
    align-items: baseline;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    border: 0;
    border-left: 1px solid var(--line);
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    transition:
      color var(--dur) var(--ease),
      border-color var(--dur) var(--ease);
  }

  .sections button:hover {
    color: var(--ink);
    border-left-color: var(--line-strong);
  }

  .sections button:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 2px;
  }

  .sections button.active {
    color: var(--ink-strong);
    border-left-color: var(--accent);
    background: var(--bg-raised);
  }

  .sections .n {
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--faint);
  }

  .sections .active .n {
    color: var(--accent-ink);
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    display: grid;
    align-content: start;
    gap: 24px;
    min-width: 0;
    min-height: 0;
  }

  section {
    width: 100%;
    max-width: 880px;
    border: 1px solid var(--line);
    background: var(--bg-raised);
    scroll-margin-top: 24px;
  }

  h2 {
    padding: 12px 24px;
    border-bottom: 1px solid var(--line);
  }

  h2:focus {
    outline: none;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 14px 24px;
  }

  .row + .row,
  .row + .split,
  .split + .row {
    border-top: 1px solid var(--line);
  }

  .name {
    color: var(--ink);
  }

  .hint {
    font-size: 13px;
    color: var(--muted);
  }

  .row.column {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }

  .slider {
    width: 280px;
  }

  .split {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 280px;
    align-items: center;
  }

  .split .slider {
    width: 220px;
  }

  .live {
    margin: 0;
    padding: 14px 24px 14px 0;
    display: grid;
    gap: 8px;
  }

  .live figcaption {
    text-align: center;
  }

  .priority {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 4px;
  }

  .priority li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border: 1px solid var(--line);
    background: var(--bg-sunken);
  }

  .priority .n {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--accent-ink);
    width: 16px;
  }

  .priority .device {
    flex: 1;
    color: var(--ink);
  }

  .priority .missing .device {
    color: var(--faint);
  }

  .priority .empty {
    font-size: 13px;
    color: var(--muted);
  }

  .confirm {
    color: var(--ink);
  }

  .licenses {
    text-align: right;
    max-width: 460px;
  }

  .dictation-look {
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 8;
    background: var(--bg);
    background-image:
      linear-gradient(var(--scanline) 1px, transparent 1px),
      linear-gradient(90deg, var(--scanline) 1px, transparent 1px);
    background-size: 12px 12px;
  }

  .dictation-look > span {
    width: 46%;
  }

  .dictation-look.analog > span {
    width: 52%;
  }
</style>
