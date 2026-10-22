// One Sketch tooltip for the whole page (PRD §9.11, UIP-002). `{@attach tip(text, shortcut)}`
// replaces `title`: hover shows it after a delay, keyboard focus shows it at once, Escape,
// a press or leaving hides it. One `TooltipLayer` per page draws it with Bits UI.
import type { Attachment } from 'svelte/attachments';

export const TOOLTIP_ID = 'macafon-tooltip';
/** Hover delay before the first tooltip; moving to another control within the grace shows at once. */
const DELAY_MS = 500;
const GRACE_MS = 300;

type Tip = { text: string; shortcut: string | null };
type Anchor = HTMLElement | SVGElement;

class TooltipState {
  anchor = $state<Anchor | null>(null);
  text = $state('');
  shortcut = $state<string | null>(null);
  open = $state(false);
  /** The element the tooltip describes: the anchor, or its focused descendant (a slider). */
  #described: Element | null = null;

  show(node: Anchor, tip: Tip, described: Element = node) {
    if (this.#described && this.#described !== described) describe(this.#described, false);
    this.anchor = node;
    this.text = tip.text;
    this.shortcut = tip.shortcut;
    this.open = true;
    this.#described = described;
    describe(described, true);
  }

  /** New text for the tooltip that shows now, keeping what it describes. */
  update(tip: Tip) {
    this.text = tip.text;
    this.shortcut = tip.shortcut;
  }

  hide() {
    if (!this.open) return;
    this.open = false;
    lastClosed = performance.now();
    if (this.#described) describe(this.#described, false);
    this.#described = null;
  }
}

export const tooltip = new TooltipState();

const tips = new WeakMap<Anchor, Tip | null>();
const wired = new WeakSet<Anchor>();
let timer: ReturnType<typeof setTimeout> | undefined;
let lastClosed = -Infinity;

/** Points `aria-describedby` at the tooltip while it shows for `node`. */
function describe(node: Element, on: boolean) {
  const ids = (node.getAttribute('aria-describedby') ?? '').split(/\s+/).filter((id) => id && id !== TOOLTIP_ID);
  if (on) ids.push(TOOLTIP_ID);
  if (ids.length) node.setAttribute('aria-describedby', ids.join(' '));
  else node.removeAttribute('aria-describedby');
}

function cancel() {
  clearTimeout(timer);
  timer = undefined;
}

function hideFor(node: Anchor) {
  cancel();
  if (tooltip.anchor === node) tooltip.hide();
}

function wire(node: Anchor) {
  node.addEventListener('pointerenter', (event) => {
    if ((event as PointerEvent).pointerType === 'touch') return;
    const tip = tips.get(node);
    if (!tip) return;
    cancel();
    if (tooltip.open || performance.now() - lastClosed < GRACE_MS) tooltip.show(node, tip);
    else timer = setTimeout(() => {
      const current = tips.get(node);
      if (current && node.isConnected) tooltip.show(node, current);
    }, DELAY_MS);
  });
  node.addEventListener('pointerleave', () => hideFor(node));
  node.addEventListener('pointerdown', () => hideFor(node));
  node.addEventListener('focusin', (event) => {
    const tip = tips.get(node);
    // Only keyboard focus: a click focuses too and must not pop a tooltip under the pointer.
    if (tip && event.target instanceof Element && event.target.matches(':focus-visible')) {
      cancel();
      tooltip.show(node, tip, event.target);
    }
  });
  node.addEventListener('focusout', () => hideFor(node));
}

/** A tooltip on this element; empty `text` shows none. `shortcut` is shown as a key. */
export function tip(text: string | null | undefined, shortcut?: string | null): Attachment<Anchor> {
  return (node) => {
    const value = text ? { text, shortcut: shortcut || null } : null;
    tips.set(node, value);
    if (!wired.has(node)) {
      wire(node);
      wired.add(node);
    }
    // A label that changes while it shows (a countdown, a new state) updates in place.
    if (tooltip.anchor === node && tooltip.open) {
      if (value) tooltip.update(value);
      else hideFor(node);
    }
    return () => {
      // The attachment re-runs when its text changes; only a removed element hides it.
      queueMicrotask(() => {
        if (!node.isConnected) hideFor(node);
      });
    };
  };
}

if (typeof window !== 'undefined') {
  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && tooltip.open) {
      cancel();
      tooltip.hide();
    }
  }, true);
  window.addEventListener('blur', () => {
    cancel();
    tooltip.hide();
  });
  window.addEventListener('scroll', () => {
    cancel();
    tooltip.hide();
  }, true);
}
