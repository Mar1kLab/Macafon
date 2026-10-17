import { expect, it } from 'vitest';
import config from '../../../src-tauri/tauri.conf.json';

it('lets the main WebView handle HTML5 mixer drags on Windows', () => {
  // Tauri's native file-drop handler intercepts HTML5 drag/drop in WebView2.
  // The runtime main-window builder reads this configuration even with create: false.
  const main = config.app.windows.find((window: { label: string }) => window.label === 'main');
  expect(main?.dragDropEnabled).toBe(false);
});
