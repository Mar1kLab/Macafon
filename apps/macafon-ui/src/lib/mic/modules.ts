// Chain modules in signal order (PRD §6.3) with their palette colours (design system §2.1).

import type { MessageKey } from '../i18n';
import type { ChainSettings } from '../ipc/protocol';
import type { MeterKey } from '../state/meters';

export type ModuleId = Exclude<keyof ChainSettings, 'muted' | 'bypass' | 'output_gain_db'>;

export interface ModuleInfo {
  id: ModuleId;
  number: string;
  color: string;
  title: MessageKey;
  hint: MessageKey;
  reduction?: MeterKey;
}

export const MODULES: Record<ModuleId, ModuleInfo> = {
  gain: { id: 'gain', number: '01', color: 'blue', title: 'modules.gain.title', hint: 'modules.gain.hint' },
  high_pass: {
    id: 'high_pass',
    number: '02',
    color: 'lime',
    title: 'modules.high_pass.title',
    hint: 'modules.high_pass.hint',
  },
  noise_suppression: {
    id: 'noise_suppression',
    number: '03',
    color: 'violet',
    title: 'modules.noise_suppression.title',
    hint: 'modules.noise_suppression.hint',
  },
  expander: {
    id: 'expander',
    number: '04',
    color: 'lime',
    title: 'modules.expander.title',
    hint: 'modules.expander.hint',
    reduction: 'expander_reduction_db',
  },
  eq: { id: 'eq', number: '05', color: 'teal', title: 'modules.eq.title', hint: 'modules.eq.hint' },
  compressor: {
    id: 'compressor',
    number: '06',
    color: 'amber',
    title: 'modules.compressor.title',
    hint: 'modules.compressor.hint',
    reduction: 'compressor_reduction_db',
  },
  de_esser: {
    id: 'de_esser',
    number: '07',
    color: 'pink',
    title: 'modules.de_esser.title',
    hint: 'modules.de_esser.hint',
    reduction: 'de_esser_reduction_db',
  },
  limiter: {
    id: 'limiter',
    number: '08',
    color: 'orange',
    title: 'modules.limiter.title',
    hint: 'modules.limiter.hint',
    reduction: 'limiter_reduction_db',
  },
};
