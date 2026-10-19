// The Analog popup's VU needle (PRD §9.9.3): a second-order spring with VU ballistics — 99 % of
// a step in 300 ms with about 1.5 % overshoot — on a scale where 0 VU is −18 dBFS RMS.

export const VU_MIN = -20;
export const VU_MAX = 3;
/** CAL-005 speech target: a calibrated voice swings around 0 VU. */
export const VU_REFERENCE_DBFS = -18;

/** Overshoot of e^(−ζπ/√(1−ζ²)) = 1.5 %. */
const DAMPING = 0.8;
/** 1 % settling time 4.6 / (ζ·ωn) = 300 ms. */
const NATURAL_RAD_S = 4.6 / (DAMPING * 0.3);

/** Needle travel 0…1 for a VU reading: the scale is linear in voltage, as on real meters. */
export function vuToTravel(vu: number): number {
  const volts = (db: number) => 10 ** (db / 20);
  const clamped = Math.min(VU_MAX, Math.max(VU_MIN, vu));
  return (volts(clamped) - volts(VU_MIN)) / (volts(VU_MAX) - volts(VU_MIN));
}

export function dbfsToVu(rmsDbfs: number): number {
  return rmsDbfs - VU_REFERENCE_DBFS;
}

export class Needle {
  position = 0;
  private velocity = 0;

  /** Advance by `dtS` toward `target` travel; long frames are split so the spring stays stable. */
  step(target: number, dtS: number): number {
    let left = Math.min(dtS, 0.25);
    while (left > 0) {
      const dt = Math.min(left, 1 / 240);
      const accel = NATURAL_RAD_S * NATURAL_RAD_S * (target - this.position) - 2 * DAMPING * NATURAL_RAD_S * this.velocity;
      this.velocity += accel * dt;
      this.position += this.velocity * dt;
      left -= dt;
    }
    return this.position;
  }

  /** Reduced motion: the needle still moves, without overshoot. */
  settle(target: number, dtS: number): number {
    this.velocity = 0;
    this.position += (target - this.position) * Math.min(1, dtS * 12);
    return this.position;
  }
}
