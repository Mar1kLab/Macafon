#!/usr/bin/env python3
"""Physical input channels, individual subscription meters and stable-device profiles."""
import array
import json
import math
import time
from m1_private import Client, PrivateGraph, smoke


def main():
    with PrivateGraph() as graph:
        graph.daemon()
        def source(name, stereo, peak):
            frames = 48000 * 90
            data = array.array('f')
            for sample in range(frames):
                tone = peak * math.sin(2 * math.pi * 440 * sample / 48000)
                if stereo:
                    data.extend((0.0, tone))
                else:
                    data.append(tone)
            path = graph.directory / (name + '.raw')
            path.write_bytes(data.tobytes())
            return graph.spawn(['pw-cat', '--playback', '--raw', '--format', 'f32', '--rate', '48000',
                '--channels', '2' if stereo else '1', '--channel-map', 'FL,FR' if stereo else 'MONO',
                '--target', '0', '--latency', '128', '--properties',
                f'node.name={name} node.description={name} media.class=Audio/Source adapter.auto-port-config={{ mode=dsp position=preserve }}', path])
        primary = source('test_microphone', True, 0.4)
        source('test_backup', False, 0.2)
        client = Client(graph.socket_path)
        smoke.wait_for(lambda: client.request('mic.get')['input_connected'], 'primary input')
        devices = smoke.wait_for(lambda: (items if len(items := client.request('devices.list')) == 2 else None), 'two physical inputs')
        assert len(devices) == 2 and all(device.get('stable_id') for device in devices), devices
        def probes():
            return [item for item in json.loads(graph.run('pw-dump'))
                if item.get('info', {}).get('props', {}).get('node.name') == 'macafon_input_meter']
        assert not probes(), 'input probes run with picker closed'
        client.request('devices.subscribe')
        smoke.wait_for(lambda: len(probes()) == 2, 'two physical input probes')
        peaks = {}
        deadline = time.monotonic() + 7
        while time.monotonic() < deadline and len(peaks) < 2:
            event = json.loads(client.reader.readline())
            if event.get('event') == 'devices.meter' and event['data']['peak_dbfs'] > -40:
                peaks[event['data']['name']] = event['data']['peak_dbfs']
        assert abs(peaks['test_microphone'] - 20 * math.log10(0.4)) < 1, peaks
        assert abs(peaks['test_backup'] - 20 * math.log10(0.2)) < 1, peaks
        client.request('devices.unsubscribe')
        smoke.wait_for(lambda: not probes(), 'picker close tears down all probes')
        for channel, peak in [('Left', 0.0), ('Right', 0.4), ('Mix', 0.2), ('Auto', 0.4)]:
            client.request('mic.set_input_channel', {'channel': channel})
            smoke.wait_for(lambda: client.request('mic.get')['input_connected'], channel + ' reconnected')
            # The PipeWire control timer applies channel selection every 100 ms.
            time.sleep(0.2)
            client.request('ab.record', {'max_seconds': 3})
            smoke.wait_for(lambda: (state := client.request('ab.get'))['recording'] is None and state['clip_ms'] is not None, channel + ' recording')
            actual = max(client.request('ab.waveform', {'points': 64})['a'])
            assert abs(actual - peak) < 0.02, (channel, actual, peak)
        settings = client.request('mic.get')['settings']
        settings['gain']['gain_db'] = 6.0
        client.request('mic.set', {'settings': settings})
        client.request('mic.set_fallback_device', {'device': 'test_backup'})
        # MIC-002: unplugging is announced with the new list, without polling.
        watcher = Client(graph.socket_path, 'watcher')
        graph.stop(primary)
        deadline = time.monotonic() + 5
        while True:
            assert time.monotonic() < deadline, 'no devices.changed after unplug'
            event = json.loads(watcher.reader.readline())
            if event.get('event') == 'devices.changed':
                assert [device['name'] for device in event['data']] == ['test_backup'], event
                break
        watcher.close()
        smoke.wait_for(lambda: client.request('mic.get')['active_device'] == 'test_backup', 'fallback input')
        settings = client.request('mic.get')['settings']
        settings['gain']['gain_db'] = -3.0
        client.request('mic.set', {'settings': settings})
        source('test_microphone', True, 0.4)
        smoke.wait_for(lambda: client.request('mic.get')['active_device'] == 'test_microphone', 'restored primary input')
        state = client.request('mic.get')
        assert state['settings']['gain']['gain_db'] == 6.0, state
        assert state.get('input_channel', 'Auto') == 'Auto', state
        client.close()
        observer = Client(graph.socket_path)
        observer.request('devices.subscribe')
        smoke.wait_for(lambda: len(probes()) == 2, 'reopened picker')
        observer.close()
        smoke.wait_for(lambda: not probes(), 'client disconnect removes probes')
        print('inputs: devices.changed on unplug, separate stereo/mono meters, subscription teardown, left/right/mix/auto and fallback profile restoration passed')


if __name__ == '__main__':
    main()
