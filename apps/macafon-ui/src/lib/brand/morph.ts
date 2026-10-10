import { contours } from './geometry';

/** Rubber warp used by the asset exporter as well as the live logo. */
export function warp(x: number, y: number, amount: number): readonly [number, number] {
  const t = Math.max(0, Math.min(1, (113 - y) / 103));
  return [
    x + amount * (52 * t ** 1.7 + 3 * Math.sin(Math.PI * t)),
    y + amount * (32 * t * t + (x - 48) * 0.9 * t),
  ];
}

export function logoPaths(amount: number): { d: string; width: number }[] {
  return contours.map(({ width, contours: paths }) => ({
    width,
    d: paths.map((points) => points.map(([x, y], i) => {
      const [px, py] = warp(x, y, amount);
      return `${i === 0 ? 'M' : 'L'}${px.toFixed(3)},${py.toFixed(3)}`;
    }).join(' ')).join(' '),
  }));
}
