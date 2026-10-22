// Animated Lucide icons adapted from Moving Icons (@jis3r/icons, MIT, see LICENSE here). They are
// decorative: the control around them carries the label. `animate` is driven by that control —
// its hover, or a short flash on a state change — and reduced motion turns every one of them off.
export interface AnimatedIconProps {
  size?: number;
  strokeWidth?: number;
  animate?: boolean;
}
