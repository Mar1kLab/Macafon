// Mixer state and requests, independent of the mock microphone transport.
import { DaemonError } from '../client';
import { CHANNEL_TEMPLATES, MAX_CHANNELS } from '../dsp';
import type { ChannelSettings, ChannelState, Event, MixerMeters, MixerState } from '../protocol';
import { channel, initialMixer } from './fixtures';
import { channelLatency, clone } from './helpers';
import { SILENT, voiceLevel } from './signals';

export class MockMixer {
  readonly state: MixerState;
  /** Apps lent to the default channel while their own is off, by app key. */
  private lent = new Map<string, string>();

  constructor(enabled: boolean, private started: number, private emit: (event: Event) => void) {
    this.state = initialMixer(enabled);
  }

  call(method: string, p: Record<string, unknown>): MixerState {
    switch (method) {
      case 'mixer.get':
        return clone(this.state);
      case 'mixer.set_enabled':
        this.state.enabled = Boolean(p.enabled);
        if (this.state.enabled && this.state.apps.length === 0) this.state.apps = initialMixer(true).apps;
        this.state.output = this.state.enabled ? (this.state.output_priority[0] ?? this.state.outputs[0]!.name) : null;
        return this.changed();
      case 'mixer.set_master':
        if (typeof p.volume_pct === 'number') this.state.master_volume_pct = p.volume_pct;
        if (typeof p.muted === 'boolean') this.state.master_muted = p.muted;
        return this.changed();
      case 'mixer.set_output': {
        const device = (p.device as string | null) ?? null;
        this.state.output_priority = device ? [device, ...this.state.output_priority.filter((d) => d !== device)] : [];
        this.state.output = this.state.enabled ? (device ?? this.state.outputs[0]!.name) : null;
        return this.changed();
      }
      case 'mixer.set_output_priority':
        this.state.output_priority = p.devices as string[];
        return this.changed();
      case 'mixer.set_auto_classify':
        this.state.auto_classify = Boolean(p.enabled);
        return this.changed();
      case 'channel.create': {
        if (this.state.channels.length >= MAX_CHANNELS) throw new DaemonError('invalid_params', 'too many channels');
        let n = 1;
        while (this.state.channels.some((c) => c.id === `ch-${n}`)) n++;
        this.state.channels.push(
          channel(`ch-${n}`, String(p.name), String(p.color), (p.icon as string | null) ?? 'speaker', CHANNEL_TEMPLATES.custom),
        );
        return this.changed();
      }
      case 'channel.update': {
        const target = this.channel(String(p.id));
        const patch = p.patch as Partial<ChannelState>;
        if (patch.enabled === false && target.id === this.state.default_channel) {
          throw new DaemonError('invalid_params', 'the default channel cannot be switched off');
        }
        Object.assign(target, patch);
        if (patch.enabled === false) {
          for (const app of this.state.apps.filter((a) => a.channel === target.id)) {
            this.lent.set(app.key, target.id);
            app.channel = this.state.default_channel;
          }
        } else if (patch.enabled === true) {
          for (const app of this.state.apps.filter((a) => this.lent.get(a.key) === target.id)) {
            this.lent.delete(app.key);
            app.channel = target.id;
          }
        }
        return this.changed();
      }
      case 'channel.delete': {
        const id = String(p.id);
        if (id === this.state.default_channel) throw new DaemonError('invalid_params', 'the default channel cannot be deleted');
        this.state.channels = this.state.channels.filter((c) => c.id !== id);
        this.state.apps.forEach((a) => a.channel === id && (a.channel = this.state.default_channel));
        return this.changed();
      }
      case 'channel.set': {
        const target = this.channel(String(p.id));
        target.settings = clone(p.settings as ChannelSettings);
        target.latency_samples = channelLatency(target.settings);
        return this.changed();
      }
      case 'channel.adjust_volume': {
        const target = this.channel(String(p.id));
        target.settings.volume_pct = Math.min(Math.max(target.settings.volume_pct + Number(p.delta_pct), 0), 100);
        return this.changed();
      }
      case 'channel.toggle_mute': {
        const target = this.channel(String(p.id));
        target.settings.muted = !target.settings.muted;
        return this.changed();
      }
      case 'channel.set_default':
        if (!this.channel(String(p.id)).enabled) throw new DaemonError('invalid_params', 'the channel is off');
        this.state.default_channel = String(p.id);
        return this.changed();
      case 'channel.reorder': {
        const ids = p.ids as string[];
        this.state.channels.sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id));
        return this.changed();
      }
      case 'apps.move': {
        if (!this.channel(String(p.channel)).enabled) throw new DaemonError('invalid_params', 'the channel is off');
        this.lent.delete(String(p.app_key));
        const target = this.state.apps.find((a) => a.key === p.app_key);
        if (target) {
          target.channel = String(p.channel);
          target.managed = true;
        }
        return this.changed();
      }
      case 'apps.set_managed': {
        const target = this.state.apps.find((a) => a.key === p.app_key);
        if (target) target.managed = Boolean(p.managed);
        return this.changed();
      }
      case 'apps.set_volume': {
        const target = this.state.apps.find((a) => a.key === p.app_key);
        if (target) target.volume_pct = Number(p.volume_pct);
        return this.changed();
      }
      case 'apps.forget':
        this.state.apps = this.state.apps.filter((a) => a.key !== p.app_key || a.streams > 0);
        return this.changed();
      default:
        throw new DaemonError('bad_request', `unknown method ${method}`);
    }
  }

  private channel(id: string): ChannelState {
    const found = this.state.channels.find((c) => c.id === id);
    if (!found) throw new DaemonError('not_found', `no channel ${id}`);
    return found;
  }

  private changed(): MixerState {
    this.emit({ event: 'mixer.changed', data: clone(this.state) });
    return clone(this.state);
  }

  // --- Meters -----------------------------------------------------------------------------

  /** Channel peaks after their faders; voices come and go, music keeps going. */
  meters(): MixerMeters {
    const t = (performance.now() - this.started) / 1000;
    const master = this.state.master_muted ? 0 : (this.state.master_volume_pct / 100) ** 3;
    const source: Record<string, number> = {
      main: -14 + 4 * Math.sin(t * 2.1),
      voice: voiceLevel(t + 1.7, -70) + 14,
      music: -9 + 3 * Math.abs(Math.sin(t * 7.3)),
    };
    const channels = this.state.channels.map((c) => {
      const playing = this.state.apps.some((a) => a.channel === c.id && a.active);
      const gain = c.settings.muted ? 0 : (c.settings.volume_pct / 100) ** 3 * master;
      const input = playing ? (source[c.id] ?? -18) : SILENT;
      const level = playing && gain > 0 ? input + 20 * Math.log10(gain) : SILENT;
      // Before the fader: the compressor works above −30 dBFS, the limiter above its ceiling.
      const fx = !c.settings.bypass;
      const compressor = fx && c.settings.compressor.enabled ? -Math.max(0, input + 30) * 0.4 : 0;
      const limiter = fx && c.settings.limiter.enabled ? -Math.max(0, input + compressor - c.settings.limiter.ceiling_db) : 0;
      return {
        id: c.id,
        meters: {
          peak_left_dbfs: level,
          peak_right_dbfs: level - (c.id === 'music' ? 1.5 * Math.sin(t * 3) : 0.5),
          compressor_reduction_db: compressor,
          limiter_reduction_db: limiter,
        },
      };
    });
    const max = (side: 'peak_left_dbfs' | 'peak_right_dbfs') =>
      channels.reduce((peak, c) => Math.max(peak, c.meters[side]), SILENT);
    return { channels, master_left_dbfs: max('peak_left_dbfs'), master_right_dbfs: max('peak_right_dbfs') };
  }

}
