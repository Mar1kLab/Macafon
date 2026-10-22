// Where tooltips and menus portal to. The UI scale is CSS `zoom` on the root (PRD §9.8): Bits UI
// measures in screen pixels but its offsets would be zoomed again, so they go into a layer that
// undoes the zoom (`.float-layer` in base.css) and scale their own content back. A modal
// <dialog> sits in the top layer above everything, so inside one they use the dialog's layer.

let root: HTMLElement | undefined;

export function floatLayer(near?: Element | null): HTMLElement {
  const dialog = near?.closest('dialog[open]');
  const inDialog = dialog?.querySelector<HTMLElement>(':scope > .float-layer');
  if (inDialog) return inDialog;
  if (!root?.isConnected) {
    root = document.createElement('div');
    root.className = 'float-layer';
    document.body.appendChild(root);
  }
  return root;
}
