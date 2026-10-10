// One requestAnimationFrame loop for every meter and graph (60 fps, design system §5).

type Callback = (now: number) => void;

const callbacks = new Set<Callback>();
let running = false;

function loop(now: number) {
  callbacks.forEach((callback) => callback(now));
  if (callbacks.size > 0) requestAnimationFrame(loop);
  else running = false;
}

/** Run `callback` every frame until the returned function is called. */
export function onFrame(callback: Callback): () => void {
  callbacks.add(callback);
  if (!running) {
    running = true;
    requestAnimationFrame(loop);
  }
  return () => callbacks.delete(callback);
}

/** Canvas sized to its CSS box at the device pixel ratio. */
export function fitCanvas(canvas: HTMLCanvasElement): CanvasRenderingContext2D | null {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  const context = canvas.getContext('2d');
  context?.setTransform(ratio, 0, 0, ratio, 0, 0);
  return context;
}

export function cssVar(element: Element, name: string): string {
  return getComputedStyle(element).getPropertyValue(name).trim();
}
