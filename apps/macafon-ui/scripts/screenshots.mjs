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
  const cache = join(homedir(), '.cache/ms-playwright');
  const dir = existsSync(cache) ? readdirSync(cache).find((name) => name.startsWith('chromium-')) : undefined;
  if (!dir) throw new Error('no Chromium found; set CHROMIUM');
  return join(cache, dir, 'chrome-linux64/chrome');
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
    await page.locator('.hotkeys').scrollIntoViewIfNeeded();
    await page.waitForTimeout(400);
  },
  'hotkeys-unavailable': async (page) => {
    await page.goto(`${base}?screen=settings&hotkeys=unavailable`);
    await page.waitForSelector('.hotkeys li');
    await page.locator('.hotkeys').scrollIntoViewIfNeeded();
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
  'settings-default': async (page) => {
    await page.goto(`${base}?screen=settings`);
    await page.waitForSelector('section');
    await page.getByRole('button', { name: 'Make default' }).click();
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
  await page.screenshot({ path: file });
  console.log(file, errors.length ? `errors: ${errors.join(' | ')}` : '');
  await page.close();
}
await browser.close();
