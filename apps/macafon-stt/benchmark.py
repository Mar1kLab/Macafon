"""Local framed-worker benchmark. Fixtures and results remain outside the repository."""
import argparse, ctypes, json, os, pathlib, re, struct, subprocess, time, wave
from ctypes import wintypes


def send(p, value, kind=1):
    data = json.dumps(value).encode() if kind == 1 else value
    p.stdin.write(struct.pack('<IB', len(data) + 1, kind) + data)
    p.stdin.flush()


def receive(p):
    header = p.stdout.read(4)
    if len(header) != 4:
        raise RuntimeError('worker exited: ' + str(p.poll()))
    size, = struct.unpack('<I', header)
    data = p.stdout.read(size)
    assert data[0] == 1
    result = json.loads(data[1:])
    if result['type'] == 'error':
        raise RuntimeError(result)
    return result


def scheduling(p, eco, normal):
    k = ctypes.WinDLL('kernel32', use_last_error=True)
    k.OpenProcess.restype = wintypes.HANDLE
    k.SetPriorityClass.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    k.SetProcessInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
    k.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = k.OpenProcess(0x0200, False, p.pid)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        state = (wintypes.DWORD * 3)(1, 1, int(eco))
        if not k.SetPriorityClass(handle, 0x20 if normal else 0x4000):
            raise ctypes.WinError(ctypes.get_last_error())
        if not k.SetProcessInformation(handle, 4, state, ctypes.sizeof(state)):
            raise ctypes.WinError(ctypes.get_last_error())
    finally:
        k.CloseHandle(handle)


def memory(p):
    class Counters(ctypes.Structure):
        _fields_ = [('cb', wintypes.DWORD), ('faults', wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in ['peak', 'working', 'pp', 'p', 'np', 'n', 'page', 'peak_page']]
    k = ctypes.WinDLL('kernel32', use_last_error=True)
    k.OpenProcess.restype = wintypes.HANDLE
    k.CloseHandle.argtypes = [wintypes.HANDLE]
    ps = ctypes.WinDLL('psapi', use_last_error=True)
    ps.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD]
    handle = k.OpenProcess(0x410, False, p.pid)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        if not ps.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
            raise ctypes.WinError(ctypes.get_last_error())
        return dict(ram_bytes=counters.working, peak_ram_bytes=counters.peak)
    finally:
        k.CloseHandle(handle)


def run(args, model, threads, eco, normal, seconds, fixed=True):
    engine = 'parakeet' if model.startswith('parakeet') else 'whisper'
    with wave.open(str(args.models / 'en.wav')) as wav:
        assert (wav.getnchannels(), wav.getframerate(), wav.getsampwidth()) == (1, 16000, 2)
        data = wav.readframes(wav.getnframes() if seconds == 10 else 3 * 16000)
    pcm = b''.join(struct.pack('<f', s / 32768) for s, in struct.iter_unpack('<h', data))
    with open(args.output.with_suffix('.native.log'), 'ab') as log:
        p = subprocess.Popen([str(args.exe.resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log)
        try:
            start = time.perf_counter()
            send(p, dict(type='load', engine=engine, model_dir=str(args.models / engine / model),
                         model_id=model, threads=threads, gpu=False, efficiency=False))
            assert receive(p)['type'] == 'loading'
            ready = receive(p)
            assert ready['type'] == 'ready'
            load_ms = round((time.perf_counter() - start) * 1000)
            scheduling(p, eco, normal)
            results = []
            for _ in range(args.repeats + 1):
                send(p, dict(type='begin'))
                send(p, pcm, 2)
                start = time.perf_counter()
                send(p, dict(type='finish', language=dict(policy='fixed' if fixed else 'auto', fixed='en', layout=None, last_cyrillic=None), vocabulary=[]))
                result = receive(p)
                results.append(dict(wall_ms=round((time.perf_counter()-start)*1000), **memory(p), **result))
            row = dict(model=model, threads=threads, eco=eco, normal=normal, seconds=len(data)/32000,
                       fixed=fixed, load_ms=load_ms, ready_ram=ready['ram_bytes'], runs=results)
            print(json.dumps(row), flush=True)
            with args.output.open('a') as out:
                out.write(json.dumps(row) + '\n')
        finally:
            p.kill()
            p.wait()


def main():
    a = argparse.ArgumentParser(__doc__)
    a.add_argument('--exe', type=pathlib.Path, default=pathlib.Path('target/release/macafon-stt.exe'))
    a.add_argument('--models', type=pathlib.Path, default=pathlib.Path(os.environ['TEMP']) / 'macafon-stt-models')
    a.add_argument('--output', type=pathlib.Path, required=True)
    a.add_argument('--phase', choices=['scheduling', 'threads', 'final', 'auto', 'tune'], required=True)
    a.add_argument('--repeats', type=int, default=2)
    args = a.parse_args()
    models = ['whisper-large-v3-turbo-q5_0', 'whisper-small-q5_1', 'parakeet-tdt-0.6b-v3-int8']
    for model in models:
        if args.phase == 'tune':
            engine = 'parakeet' if model.startswith('parakeet') else 'whisper'
            variants = [[], ['--flash-attention'], ['--single-segment'], ['--audio-ctx', '500'],
                        ['--audio-ctx', '500', '--flash-attention'], ['--beam-size', '5']]
            if engine == 'parakeet':
                variants = [['--vad-threads', '6'], ['--vad-threads', '1']]
            for flags in variants:
                command = [str(args.exe.resolve()), 'transcribe', str(args.models / 'en.wav'),
                           '--engine', engine, '--model-dir', str(args.models / engine / model),
                           '--threads', '6', '--lang', 'en', '--repeat', str(args.repeats + 1), *flags]
                if engine == 'whisper':
                    command.extend(['--vad-threads', '6'])
                result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=300)
                log = re.sub(r'\x1b\[[0-9;]*m', '', result.stderr)
                row = dict(model=model, flags=flags, runs=[json.loads(s) for s in result.stdout.splitlines()],
                           stages=[dict(re.findall(r'(\w+)=(\d+)', s)) for s in log.splitlines()
                                   if 'stages' in s or 'model measurement' in s],
                           backend=[s for s in log.splitlines() if 'backend' in s])
                with args.output.open('a') as out:
                    out.write(json.dumps(row) + '\n')
                print(json.dumps(row), flush=True)
        elif args.phase == 'scheduling':
            for eco, normal in [(True, False), (False, False), (False, True), (True, True)]:
                run(args, model, 7, eco, normal, 10)
        elif args.phase == 'threads':
            for threads in [4, 6, 8, 12]:
                run(args, model, threads, False, False, 10)
        else:
            for seconds in [10, 3]:
                run(args, model, 6, False, False, seconds, fixed=args.phase != 'auto')


if __name__ == '__main__':
    main()
