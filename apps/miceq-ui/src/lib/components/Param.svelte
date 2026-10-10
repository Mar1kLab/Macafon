<script lang="ts">
  // Ruler slider bound to a parameter spec of miceq-dsp: range, default, unit and step come
  // from the generated SPECS table, never from the UI (AGENTS.md §4).
  import { SPECS } from '../ipc/dsp';
  import { formatValue } from '../dsp/format';
  import RulerSlider from './RulerSlider.svelte';

  type SpecKey = keyof typeof SPECS;

  interface Props {
    spec: SpecKey;
    label: string;
    value: number;
    onchange: (value: number) => void;
    color?: string;
    disabled?: boolean;
    hint?: string;
    layout?: 'stacked' | 'row';
    /** Default override when the spec default does not apply (calibrated thresholds). */
    defaultValue?: number;
    format?: (value: number) => string;
  }

  let { spec, label, value, onchange, color, disabled, hint, layout = 'row', defaultValue, format }: Props =
    $props();

  const s = $derived(SPECS[spec]);
  const decimals = $derived(
    s.unit === 'percent' || s.unit === 'hz' ? 0 : s.unit === 'factor' ? 2 : s.max - s.min > 100 ? 0 : 1,
  );
  const scale = $derived(s.min > 0 && (s.unit === 'hz' || s.max / s.min >= 100) ? 'log' : 'linear');
  const origin = $derived(s.unit === 'db' && s.min < 0 && s.max > 0 ? 0 : undefined);
</script>

<RulerSlider
  {value}
  min={s.min}
  max={s.max}
  step={s.step}
  {decimals}
  {scale}
  {origin}
  defaultValue={defaultValue ?? s.default}
  {label}
  format={format ?? ((v) => formatValue(v, s.unit))}
  {onchange}
  {color}
  {disabled}
  {hint}
  {layout}
/>
