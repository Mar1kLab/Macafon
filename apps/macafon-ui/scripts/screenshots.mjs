// Screenshots of the UI in mock mode at the default window size (AGENTS.md §6).
// Usage: node scripts/screenshots.mjs <out-dir> [scenario ...]; the dev server must run.
import { chromium } from 'playwright-core';
import { existsSync, mkdirSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const base = process.env.MACAFON_UI_URL ?? 'http://127.0.0.1:1420/';
const out = process.argv[2] ?? '.artifacts/screenshots';
const only = new Set(process.argv.slice(3));
mkdirSync(out, { recursive: true });

/** Chromium from the Playwright cache unless CHROMIUM is set. */
function executable() {
  if (process.env.CHROMIUM) return process.env.CHROMIUM;
  const windows = process.platform === 'win32';
  const cache = windows ? join(process.env.LOCALAPPDATA ?? homedir(), 'ms-playwright') : join(homedir(), '.cache/ms-playwright');
  const dir = existsSync(cache) ? readdirSync(cache).find((name) => name.startsWith('chromium-')) : undefined;
  if (!dir) throw new Error('no Chromium found; set CHROMIUM');
  return join(cache, dir, windows ? 'chrome-win64/chrome.exe' : 'chrome-linux64/chrome');
}

/** Click through the first-run flow up to `step` (0 = welcome). */
async function onboardingTo(page, step) {
  // A first run: the mixer is not turned on yet (CH-009).
  await page.goto(`${base}?mixer=off`);
  await page.waitForSelector('.onboarding');
  if (step === 0) return;
  await page.getByRole('button', { name: 'Get started' }).click();
  if (step === 1) return page.waitForTimeout(1800);
  if (step >= 2) await page.getByRole('button', { name: 'Next' }).click();
  if (step === 2) return page.waitForTimeout(400);
  if (step >= 3) {
    await page.getByRole('button', { name: 'Start' }).click();
    await page.waitForTimeout(10_800);
  }
  if (step === 3) return;
  await page.getByRole('button', { name: 'Next' }).click();
  if (step === 4) return page.waitForTimeout(400);
  await page.getByRole('button', { name: 'Next' }).click();
  if (step === 5) return page.waitForTimeout(400);
  await page.getByRole('button', { name: 'Skip', exact: true }).click();
  await page.waitForTimeout(400);
}

const scenarios = {
  'controls-mixer': async (page) => {
    await page.goto(`${base}?platform=windows&screen=mixer`);
    await page.waitForSelector('.channels');
    await page.locator('.toast button').first().click();
  },
  'controls-microphone': async (page) => {
    await page.goto(`${base}?platform=windows`);
    await page.waitForSelector('.chain');
  },
  'controls-settings': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings`);
    await page.waitForSelector('.hotkeys');
  },
  'controls-hotkeys': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings`);
    await page.waitForSelector('.hotkeys');
    await page.locator('.hotkeys').evaluate((element) => element.closest('section').scrollIntoView({ block: 'start' }));
  },
  'controls-offline': async (page) => {
    await page.goto(`${base}?platform=windows&daemon=down`);
    await page.waitForSelector('.unavailable');
  },
  'controls-mode': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings`);
    await page.getByRole('button', { name: 'Windows sessions', exact: true }).evaluate((element) => element.closest('section').scrollIntoView({ block: 'center' }));
  },
  'controls-popup': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings`);
    await page.getByRole('radiogroup', { name: 'Popup & overlay monitor' }).scrollIntoViewIfNeeded();
  },
  'macaron-unknown': async (page) => {
    await page.goto(`${base}?platform=windows&screen=mixer&macaron=unknown`);
    await page.getByRole('button', { name: 'Device settings', exact: true }).click();
    await page.getByRole('slider', { name: 'Volume', exact: true }).waitFor();
  },
  'macaron-live': async (page) => {
    await page.goto(`${base}?platform=windows&screen=mixer&macaron=live`);
    await page.getByRole('button', { name: 'Device settings', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[role="slider"][aria-label="Volume"]')?.getAttribute('aria-valuenow') === '81');
  },
  'macaron-known': async (page) => {
    await page.goto(`${base}?platform=windows&screen=mixer&macaron=known`);
    await page.getByRole('button', { name: 'Device settings', exact: true }).click();
    await page.getByRole('button', { name: 'High', exact: true }).click();
    await page.locator('button[aria-pressed="true"]').filter({ hasText: 'High' }).waitFor();
    await page.getByRole('button', { name: 'NOS', exact: true }).click();
    await page.locator('button[aria-pressed="true"]').filter({ hasText: 'NOS' }).waitFor();
  },
  'macaron-error': async (page) => {
    await page.goto(`${base}?platform=windows&screen=mixer&macaron=error`);
    await page.getByRole('button', { name: 'Device settings', exact: true }).click();
    await page.getByRole('button', { name: 'Mid', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'timed out' }).waitFor();
  },
  'macaron-disconnected': async (page) => {
    await page.goto(`${base}?platform=windows&screen=mixer&macaron=disconnect`);
    await page.getByRole('button', { name: 'Device settings', exact: true }).click();
    await page.getByText('Disconnected', { exact: true }).waitFor();
  },
  'macaron-absent': async (page) => {
    await page.goto(`${base}?platform=windows&screen=mixer`);
    await page.waitForSelector('.strip');
    if (await page.getByRole('button', { name: 'Device settings', exact: true }).count()) throw new Error('Controls visible without a supported DAC');
  },
  mixer: async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.waitForTimeout(1500);
  },
  'mixer-off': async (page) => {
    await page.goto(`${base}?screen=mixer&mixer=off`);
    await page.waitForSelector('.intro');
    await page.waitForTimeout(500);
  },
  'mixer-eq': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.locator('section[aria-label="Voice"]').getByRole('button', { name: /^EQ/ }).click();
    await page.waitForTimeout(900);
  },
  'mixer-fx': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.locator('section[aria-label="Voice"]').getByRole('button', { name: /^FX/ }).click();
    await page.waitForTimeout(900);
  },
  'mixer-reorder': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    // Keyboard: Music one place left; mouse: Main after Voice.
    await page.getByRole('button', { name: 'Move Music: drag, or press ← →' }).focus();
    await page.keyboard.press('ArrowLeft');
    await page
      .getByRole('button', { name: 'Move Main: drag, or press ← →' })
      .dragTo(page.locator('section[aria-label="Voice"]'), { targetPosition: { x: 170, y: 300 } });
    await page.getByRole('button', { name: 'Move Voice: drag, or press ← →' }).focus();
    await page.waitForTimeout(1200);
  },
  'mixer-app-volume': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.locator('.chip', { hasText: 'Firefox' }).click();
    for (let i = 0; i < 4; i++) await page.keyboard.press('PageDown');
    await page.waitForTimeout(600);
  },
  'mixer-channel': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.getByRole('button', { name: 'Music settings' }).click();
    await page.waitForTimeout(900);
  },
  'mixer-moved': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.locator('.chip', { hasText: 'Firefox' }).dragTo(page.locator('section[aria-label="Music"] .apps'));
    await page.waitForTimeout(1200);
  },
  'mixer-menu': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.locator('.chip', { hasText: 'Cyberpunk' }).click({ button: 'right' });
    await page.waitForTimeout(600);
  },
  'settings-mixer': async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.getByRole('button', { name: 'Add output' }).click();
    await page.getByRole('option', { name: 'Arctis Nova 7' }).click();
    await page.getByRole('button', { name: 'Add output' }).click();
    await page.getByRole('option', { name: 'Built-in Speakers' }).click();
    await page.getByText('Output priority').scrollIntoViewIfNeeded();
    await page.waitForTimeout(500);
  },
  doctor: async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('.doctor li');
    await page.locator('.doctor').scrollIntoViewIfNeeded();
    await page.waitForTimeout(400);
  },
  hotkeys: async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('.hotkeys li');
    // A conflict: Ctrl+Alt+M already toggles the microphone.
    await page.getByRole('button', { name: 'Set shortcut for Toggle bypass' }).click();
    await page.keyboard.press('Control+Alt+KeyM');
    await page.getByRole('button', { name: 'Set shortcut for Voice: Toggle mute' }).click();
    await page.keyboard.down('Control');
    await page.keyboard.down('Shift');
    await page.locator('.hotkeys').evaluate((element) => element.closest('section').scrollIntoView({ block: 'start' }));
    await page.waitForTimeout(400);
  },
  'hotkeys-unavailable': async (page) => {
    await page.goto(`${base}?screen=settings&hotkeys=unavailable`);
    await page.waitForSelector('.hotkeys li');
    await page.locator('.hotkeys').evaluate((element) => element.closest('section').scrollIntoView({ block: 'start' }));
    await page.waitForTimeout(400);
  },
  'mixer-new': async (page) => {
    await page.goto(`${base}?screen=mixer`);
    await page.waitForSelector('.strip');
    await page.locator('button.new').click();
    await page.keyboard.type('Games');
    await page.waitForTimeout(500);
  },
  'onboarding-welcome': (page) => onboardingTo(page, 0).then(() => page.waitForTimeout(600)),
  'onboarding-mic': (page) => onboardingTo(page, 1),
  'onboarding-calibrating': async (page) => {
    await onboardingTo(page, 2);
    await page.getByRole('button', { name: 'Start' }).click();
    await page.waitForTimeout(6200);
  },
  'onboarding-result': async (page) => {
    await onboardingTo(page, 3);
    await page.getByRole('button', { name: 'Listen A/B' }).click();
    await page.waitForTimeout(2300 + 4200);
    await page.locator('.test .rec').click();
    await page.getByRole('button', { name: 'Loop' }).click();
    await page.waitForTimeout(1200);
  },
  'onboarding-use': (page) => onboardingTo(page, 4),
  'onboarding-mixer': (page) => onboardingTo(page, 5),
  'onboarding-background': (page) => onboardingTo(page, 6),
  'device-meters': async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.locator('.picker .trigger').click();
    await page.waitForTimeout(1200);
  },
  'settings-prefs': async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.getByRole('button', { name: 'Buffer size' }).click();
    await page.waitForTimeout(400);
  },
  'settings-default': async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.getByRole('switch', { name: 'System default microphone' }).click();
    await page.waitForTimeout(500);
  },
  'settings-default-on': async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.getByRole('switch', { name: 'System default microphone' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Make default' }).click();
    await page.waitForTimeout(500);
  },
  'noise-fallback': async (page) => {
    await page.goto(`${base}?overload&config=recovered`);
    await page.waitForSelector('.chain');
    await page.waitForTimeout(2600);
  },
  microphone: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.waitForTimeout(1500);
  },
  advanced: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.locator('section[aria-label="Compressor"] .advanced-toggle').click();
    await page.locator('section[aria-label="Compressor"] [role="slider"]').nth(1).focus();
    await page.keyboard.press('PageDown');
    await page.waitForTimeout(1200);
  },
  eq: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.locator('section[aria-label="EQ"] .expand').click();
    await page.locator('.point').nth(3).focus();
    for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowUp');
    await page.waitForTimeout(900);
  },
  calibration: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Calibrate' }).first().click();
    await page.getByRole('button', { name: 'Start' }).click();
    await page.waitForTimeout(6200);
  },
  'calibration-result': async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Calibrate' }).first().click();
    await page.getByRole('button', { name: 'Start' }).click();
    await page.waitForTimeout(10_800);
  },
  ab: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.locator('.test .rec').click();
    await page.waitForTimeout(2300 + 5200);
    await page.locator('.test .rec').click();
    await page.getByRole('button', { name: 'Loop' }).click();
    await page.waitForTimeout(1600);
  },
  disconnected: async (page) => {
    await page.goto(`${base}?mic=disconnected`);
    await page.waitForSelector('.chain');
    await page.waitForTimeout(800);
  },
  bypass: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Bypass', exact: true }).click();
    await page.waitForTimeout(800);
  },
  'quit-confirm': async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.getByRole('button', { name: 'Quit Macafon' }).click();
    await page.waitForTimeout(500);
  },
  muted: async (page) => {
    await page.goto(base);
    await page.waitForSelector('.chain');
    await page.getByRole('button', { name: 'Mute' }).click();
    await page.waitForTimeout(1200);
  },
  settings: async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.waitForTimeout(600);
  },
  'settings-popup': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings&section=popup`);
    await page.waitForSelector('#settings-popup');
    await page.waitForTimeout(900);
  },
  'settings-popup-bar': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings&section=popup`);
    await page.waitForSelector('#settings-popup');
    await page.getByRole('radio', { name: /Windows 11/ }).click();
    await page.waitForTimeout(300);
    await page.getByRole('radio', { name: 'Top', exact: true }).click();
    await page.waitForTimeout(300);
    await page.locator('#settings-popup').getByRole('slider').focus();
    for (let i = 0; i < 3; i += 1) await page.keyboard.press('PageUp');
    await page.locator('#settings-popup .live').evaluate((element) => element.scrollIntoView({ block: 'center' }));
    await page.waitForTimeout(900);
  },
  'settings-overlay': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings&section=popup`);
    await page.waitForSelector('#settings-popup');
    await page.locator('#settings-popup .row.column').last().evaluate((element) => element.scrollIntoView({ block: 'center' }));
    await page.waitForTimeout(900);
  },
  'overlay-collapsed': async (page) => {
    await page.goto(`${base}?platform=windows&view=quick-panel&mode=overlay`);
    await page.waitForSelector('.overlay .sounds');
    await page.waitForTimeout(600);
  },
  'overlay-sounds': async (page) => {
    await page.goto(`${base}?platform=windows&view=quick-panel&mode=overlay`);
    await page.waitForSelector('.overlay .sounds');
    await page.getByRole('button', { name: 'Show the soundpad sounds' }).click();
    await page.waitForSelector('.pads');
    await page.waitForTimeout(600);
  },
  'overlay-sounds-playing': async (page) => {
    // The sounds stay open from the previous opening; a pad plays with its transport.
    await page.goto(`${base}?platform=windows&view=quick-panel&mode=overlay`);
    await page.waitForSelector('.pads');
    await page.getByRole('button', { name: 'Play Elevator loop' }).click();
    await page.waitForTimeout(2500);
  },
  'overlay-sketch-list': async (page) => {
    // The sketch look with the sounds in a list and the player below them.
    await page.goto(`${base}?platform=windows&view=quick-panel&mode=overlay&look=sketch&sounds=list&controls=bottom`);
    await page.waitForSelector('.pads');
    await page.getByRole('button', { name: 'Play Elevator loop' }).click();
    await page.waitForTimeout(2500);
  },
  'overlay-sketch-grid': async (page) => {
    await page.goto(`${base}?platform=windows&view=quick-panel&mode=overlay&look=sketch`);
    await page.waitForSelector('.pads');
    await page.getByRole('button', { name: 'Play Elevator loop' }).click();
    await page.waitForTimeout(2500);
  },
  'settings-look': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings&section=popup`);
    await page.waitForSelector('#settings-popup');
    await page.waitForTimeout(900);
  },
  'settings-overlay-sounds': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings&section=popup`);
    await page.waitForSelector('#settings-popup');
    await page.getByRole('radiogroup', { name: 'Sounds as' }).scrollIntoViewIfNeeded();
    await page.waitForTimeout(900);
  },
  'overlay-sort': async (page) => {
    await page.goto(`${base}?platform=windows&view=quick-panel&mode=overlay`);
    await page.waitForSelector('.pads');
    await page.getByRole('button', { name: 'Sort' }).click();
    await page.waitForSelector('[role="listbox"]');
    await page.waitForTimeout(300);
  },
  'dropdown-small-window': async (page) => {
    // The smallest window: the headphones device list opens upward inside the side panel.
    await page.setViewportSize({ width: 960, height: 640 });
    await page.goto(`${base}?platform=windows&screen=soundpad`);
    await page.waitForSelector('.list');
    await page.getByRole('button', { name: 'Headphones device' }).click();
    await page.waitForSelector('[role="listbox"]');
    const grown = await page.evaluate(() => document.documentElement.scrollHeight > innerHeight || document.body.scrollHeight > innerHeight);
    if (grown) throw new Error('the open list made the page taller than the window');
    await page.waitForTimeout(300);
  },
  'soundpad-folder': async (page) => {
    await page.goto(`${base}?platform=windows&screen=soundpad`);
    await page.waitForSelector('.list');
    await page.waitForTimeout(600);
  },
  'settings-soundpad': async (page) => {
    await page.goto(`${base}?platform=windows&screen=settings&section=soundpad`);
    await page.waitForSelector('#settings-soundpad');
    await page.getByRole('button', { name: 'Change folder' }).click();
    await page.locator('#prompt-input').fill('D:\\Audio\\Stream sounds');
    await page.getByRole('button', { name: 'Use folder' }).click();
    await page.waitForTimeout(900);
  },
  unavailable: async (page) => {
    await page.goto(`${base}?daemon=down`);
    await page.waitForSelector('.unavailable');
    await page.waitForTimeout(300);
  },
};

const browser = await chromium.launch({ executablePath: executable() });
const context = await browser.newContext({ viewport: { width: 1180, height: 760 }, deviceScaleFactor: 1 });
const date = new Date().toISOString().slice(0, 10);
for (const [name, run] of Object.entries(scenarios)) {
  if (only.size && !only.has(name)) continue;
  const page = await context.newPage();
  // The first-run flow covers the app; only its own scenarios show it.
  await page.addInitScript((first) => {
    if (first) localStorage.removeItem('macafon.onboarding');
    else localStorage.setItem('macafon.onboarding', 'done');
  }, name.startsWith('onboarding'));
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => message.type() === 'error' && errors.push(message.text()));
  await run(page);
  const file = join(out, `${date}-ui-${name}.png`);
  await page.screenshot({ path: file, animations: 'disabled' });
  if (name === 'macaron-live') await page.locator('dialog[open]').screenshot({ path: join(out, `${date}-ui-macaron-detail.png`), animations: 'disabled' });
  console.log(file, errors.length ? `errors: ${errors.join(' | ')}` : '');
  await page.close();
}
await browser.close();
