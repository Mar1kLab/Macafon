// The UI scale (PRD §9.8) is CSS `zoom` on the root: rectangles and the window size are in
// screen pixels, while lengths set in CSS are scaled by the zoom. Convert between the two.

/** The current UI zoom, 1 at 100 %. */
export function uiZoom(): number {
  const zoom = Number.parseFloat(getComputedStyle(document.documentElement).zoom);
  return Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
}

/** Screen pixels (from getBoundingClientRect, innerHeight, pointer events) to CSS pixels. */
export function toCss(px: number): number {
  return px / uiZoom();
}
