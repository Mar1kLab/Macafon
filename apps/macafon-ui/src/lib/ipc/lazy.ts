// A transport loaded on first use: the mock daemon is only for a plain browser, so the Tauri
// bundle never parses it (it is a separate chunk that the app does not fetch).

import type { DaemonStatus, Transport } from './client';
import type { Event } from './protocol';

export class LazyTransport implements Transport {
  private readonly events = new Set<(event: Event) => void>();
  private readonly statuses = new Set<(status: DaemonStatus) => void>();
  private status: DaemonStatus = { state: 'connecting' };
  private readonly ready: Promise<Transport>;

  constructor(load: () => Promise<Transport>) {
    this.ready = load().then((transport) => {
      transport.onEvent((event) => this.events.forEach((handler) => handler(event)));
      transport.onStatus((status) => {
        this.status = status;
        this.statuses.forEach((handler) => handler(status));
      });
      return transport;
    });
  }

  call(method: string, params: unknown): Promise<unknown> {
    return this.ready.then((transport) => transport.call(method, params));
  }

  onEvent(handler: (event: Event) => void): () => void {
    this.events.add(handler);
    return () => this.events.delete(handler);
  }

  onStatus(handler: (status: DaemonStatus) => void): () => void {
    this.statuses.add(handler);
    handler(this.status);
    return () => this.statuses.delete(handler);
  }

  retry(): void {
    void this.ready.then((transport) => transport.retry());
  }
}
