import { expect, it } from 'vitest';
import type { Event } from '../protocol';
import { MockMixer } from './mixer';

it('keeps app routing and channel meters consistent through a mixer editing session', () => {
  const events: Event[] = [];
  const mixer = new MockMixer(true, performance.now(), (event) => events.push(event));
  const created = mixer.call('channel.create', { name: 'Games', color: 'orange', icon: 'gamepad' });
  const id = created.channels.at(-1)!.id;
  mixer.call('apps.move', { app_key: 'bin:firefox', channel: id });
  mixer.call('channel.adjust_volume', { id, delta_pct: 30 - created.channels.at(-1)!.settings.volume_pct });
  expect(mixer.state.apps.find((app) => app.key === 'bin:firefox')?.channel).toBe(id);
  expect(mixer.state.channels.find((channel) => channel.id === id)?.settings.volume_pct).toBe(30);
  mixer.call('channel.toggle_mute', { id });
  expect(mixer.meters().channels.find((channel) => channel.id === id)?.meters.peak_left_dbfs).toBe(-120);

  mixer.call('channel.delete', { id });
  expect(mixer.state.apps.find((app) => app.key === 'bin:firefox')?.channel).toBe('main');
  expect(() => mixer.call('channel.delete', { id: 'main' })).toThrow('default channel');
  // Earlier response/event payloads remain snapshots after later state changes.
  expect(created.channels.some((channel) => channel.id === id)).toBe(true);
  const last = events.at(-1)!;
  expect(last.event).toBe('mixer.changed');
  if (last.event === 'mixer.changed') {
    expect(last.data.channels.some((channel) => channel.id === id)).toBe(false);
  }
});
