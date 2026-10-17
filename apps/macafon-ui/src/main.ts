import { mount } from 'svelte';
// Fonts are bundled (SIL OFL 1.1); nothing is loaded from the network (AGENTS.md §5).
import '@fontsource-variable/archivo';
import '@fontsource-variable/inter';
import '@fontsource-variable/geist-mono';
import '@fontsource/instrument-serif/400-italic.css';
import './styles/tokens.css';
import './styles/base.css';
import App from './App.svelte';
import QuickPanel from './lib/quick/QuickPanel.svelte';
import HotkeyOverlay from './lib/quick/HotkeyOverlay.svelte';
import { quickPanel } from './lib/state';

const target = document.getElementById('app');
if (!target) throw new Error('missing #app');

const overlay = quickPanel && new URLSearchParams(location.search).get('mode') === 'overlay';
export default mount(quickPanel ? (overlay ? HotkeyOverlay : QuickPanel) : App, { target });
