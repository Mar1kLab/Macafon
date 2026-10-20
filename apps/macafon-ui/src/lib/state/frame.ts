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

/** A canvas drawn every frame. Its CSS size is observed rather than read in each frame (a
 * layout read per meter per frame forces layout over and over), and a frame that would draw
 * the same picture again is skipped. */
export class FrameCanvas {
  /** CSS pixels. */
  width: number;
  height: number;
  private readonly observer: ResizeObserver | null = null;
  private readonly context: CanvasRenderingContext2D | null;
  private ratio = 0;
  private key: string | number | undefined;
  private resized = true;

  constructor(private readonly canvas: HTMLCanvasElement) {
    this.width = canvas.clientWidth;
    this.height = canvas.clientHeight;
    this.context = canvas.getContext('2d');
    if (typeof ResizeObserver !== 'undefined') {
      this.observer = new ResizeObserver(([entry]) => {
        const box = entry?.contentBoxSize?.[0];
        if (!box) return;
        this.width = box.inlineSize;
        this.height = box.blockSize;
        this.resized = true;
      });
      this.observer.observe(canvas);
    }
  }

  /** The cleared context in CSS pixels, or null when `key` (everything the picture depends
   * on) is the one already drawn at this size. Without a key every frame is drawn. */
  begin(key?: string | number): CanvasRenderingContext2D | null {
    const context = this.context;
    if (!context) return null;
    const ratio = window.devicePixelRatio || 1;
    if (key !== undefined && key === this.key && ratio === this.ratio && !this.resized) return null;
    this.key = key;
    if (ratio !== this.ratio || this.resized) {
      this.ratio = ratio;
      this.resized = false;
      const width = Math.max(1, Math.round(this.width * ratio));
      const height = Math.max(1, Math.round(this.height * ratio));
      if (this.canvas.width !== width || this.canvas.height !== height) {
        this.canvas.width = width;
        this.canvas.height = height;
      }
    }
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, this.width, this.height);
    return context;
  }

  dispose(): void {
    this.observer?.disconnect();
  }
}

/** Draw on `canvas` every frame until the returned function is called. */
export function drawEveryFrame(canvas: HTMLCanvasElement, draw: (surface: FrameCanvas, now: number) => void): () => void {
  const surface = new FrameCanvas(canvas);
  const stop = onFrame((now) => draw(surface, now));
  return () => {
    stop();
    surface.dispose();
  };
}

/** Canvas sized to its CSS box at the device pixel ratio, for pictures drawn on demand. */
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
