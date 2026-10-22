import { mount } from 'svelte';
// Fonts are bundled (SIL OFL 1.1); nothing is loaded from the network (AGENTS.md §5).
import '@fontsource-variable/archivo';
import '@fontsource-variable/inter';
import '@fontsource-variable/geist-mono';
import '@fontsource/instrument-serif/400-italic.css';
import './styles/tokens.css';
import './styles/base.css';
import { quickPanel } from './lib/state';

const target = document.getElementById('app');
if (!target) throw new Error('missing #app');

// Each window loads only its own UI: the warm quick panel never parses the main screens.
const { default: root } = quickPanel ? await import('./lib/quick/Panel.svelte') : await import('./App.svelte');
export default mount(root, { target });
