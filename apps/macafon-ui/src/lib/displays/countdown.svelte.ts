// The seconds left of a DISP-004 confirmation, ticking while one is pending.

export function useCountdown(deadline: () => number | null) {
  let now = $state(Date.now());
  $effect(() => {
    if (deadline() === null) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 250);
    return () => clearInterval(timer);
  });
  return {
    get seconds(): number {
      const at = deadline();
      return at === null ? 0 : Math.max(0, Math.ceil((at - now) / 1000));
    },
  };
}
