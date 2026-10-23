// Optimistic preference patches (sounddeck, dictation): one request in flight, later patches
// merge into the next one, and the daemon's replies keep the fields still being edited local.

type Patch = object;

/** `prefs` with the set (non-null) fields of `patch`. */
export function withPatch<T extends object>(prefs: T, patch: Patch): T {
  const next = { ...prefs };
  for (const [key, value] of Object.entries(patch)) {
    if (value !== undefined && value !== null) (next as Record<string, unknown>)[key] = value;
  }
  return next;
}

export class PrefsQueue<P extends Patch, S> {
  private busy = false;
  private sent: P | null = null;
  private pending: P | null = null;

  constructor(
    private readonly call: (patch: P) => Promise<S | null>,
    private readonly refetch: () => Promise<S | null | undefined>,
    private readonly apply: (state: S) => void,
    private readonly fail: (error: unknown) => void,
  ) {}

  /** The daemon's `prefs`, with the fields still being edited at their `local` values. */
  keepEditing<T extends object>(prefs: T, local: T | undefined): T {
    const next = { ...prefs };
    if (!local) return next;
    for (const [key, value] of Object.entries({ ...this.sent, ...this.pending })) {
      if (value !== null && value !== undefined) {
        (next as Record<string, unknown>)[key] = (local as Record<string, unknown>)[key];
      }
    }
    return next;
  }

  /** Sends `patch` now, or merges it into the next request while one is in flight. */
  push(patch: P): void {
    if (this.busy) {
      this.pending = { ...this.pending, ...patch };
      return;
    }
    this.busy = true;
    this.sent = patch;
    this.call(patch)
      .then((state) => {
        this.sent = null;
        if (state) this.apply(state);
      })
      .catch(async (error) => {
        this.fail(error);
        this.sent = null;
        const state = await this.refetch();
        if (state) this.apply(state);
      })
      .finally(() => {
        this.busy = false;
        const next = this.pending;
        this.pending = null;
        if (next) this.push(next);
      });
  }
}
