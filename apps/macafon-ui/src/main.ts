import { mount } from 'svelte';
// Fonts are bundled (SIL OFL 1.1); nothing is loaded from the network (AGENTS.md §5).
import '@fontsource-variable/archivo';
import '@fontsource-variable/inter';
import '@fontsource-variable/geist-mono';
import '@fontsource/instrument-serif/400-italic.css';
import './styles/tokens.css';
import './styles/base.css';
import App from './App.svelte';
import Panel from './lib/quick/Panel.svelte';
import { quickPanel } from './lib/state';

const target = document.getElementById('app');
if (!target) throw new Error('missing #app');

export default mount(quickPanel ? Panel : App, { target });
