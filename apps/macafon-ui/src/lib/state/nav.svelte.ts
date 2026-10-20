// Which screen the main window shows, the Settings section to open and where Back returns to.
import type { Screen } from '../components/Sidebar.svelte';

export type SettingsSection =
  | 'general'
  | 'audio'
  | 'outputs'
  | 'mixer'
  | 'soundpad'
  | 'hotkeys'
  | 'popup'
  | 'overlay'
  | 'dictation'
  | 'doctor'
  | 'about';

const query = new URLSearchParams(location.search);
const initial = query.get('screen');
const SCREENS: Screen[] = ['microphone', 'mixer', 'soundpad', 'dictation', 'settings'];

class Nav {
  screen = $state<Screen>(SCREENS.includes(initial as Screen) ? (initial as Screen) : 'microphone');
  /** Settings scrolls here once it shows; bumped on every request so the same one repeats. */
  section = $state<{ id: SettingsSection; seq: number } | null>(null);
  /** The screen Settings was opened from: its Back button returns there. */
  previous = $state<Exclude<Screen, 'settings'>>('microphone');

  constructor() {
    const section = query.get('section');
    if (section) this.section = { id: section as SettingsSection, seq: 0 };
  }

  go(screen: Screen) {
    if (screen === 'settings' && this.screen !== 'settings') this.previous = this.screen;
    this.screen = screen;
  }

  /** Settings at the section that configures what the current screen shows. */
  settings(id: SettingsSection) {
    this.section = { id, seq: (this.section?.seq ?? 0) + 1 };
    this.go('settings');
  }

  back() {
    this.screen = this.previous;
  }
}

export const nav = new Nav();
