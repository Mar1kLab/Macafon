// Arrow keys in a radio group (PRD §9.8): Left/Up and Right/Down select the previous or next
// radio, Home and End the first or last, and focus follows. The radios keep `tabindex` 0 on the
// checked one and -1 on the others, so Tab enters the group once.

export function radioKeys(group: HTMLElement) {
  const onkeydown = (event: KeyboardEvent) => {
    const radios = [...group.querySelectorAll<HTMLElement>('[role="radio"]:not(:disabled)')];
    const index = radios.indexOf(document.activeElement as HTMLElement);
    if (index < 0) return;
    const step: Record<string, number> = { ArrowRight: index + 1, ArrowDown: index + 1, ArrowLeft: index - 1, ArrowUp: index - 1, Home: 0, End: radios.length - 1 };
    if (!(event.key in step)) return;
    event.preventDefault();
    const next = radios[(step[event.key]! + radios.length) % radios.length];
    next?.focus();
    next?.click();
  };
  group.addEventListener('keydown', onkeydown);
  return { destroy: () => group.removeEventListener('keydown', onkeydown) };
}
