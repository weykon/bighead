// WebSocket client + reducer for ViewCommand stream.
// On connect, sends 'ready' so the server can route events back.

import type { PanelState, ViewCommand } from './types';

export type WsStatus = 'connecting' | 'open' | 'reconnecting' | 'closed';

export type WsSnapshot = {
  status: WsStatus;
  retry: number;
  maxRetries: number;
  nextRetryMs: number;
};

// Global ws instance for drag events
let globalSend: ((payload: unknown) => boolean) | null = null;

export function setGlobalWs(send: ((payload: unknown) => boolean) | null) {
  globalSend = send;
}

export function sendMove(id: string, position: [number, number, number]) {
  if (globalSend) {
    globalSend({
      type: 'drag_end',
      id,
      position,
    });
    console.log('[ws] sent drag_end:', id, position);
  }
}

export function sendViewEvent(panel_id: string, data: unknown) {
  if (globalSend) {
    return globalSend({
      type: 'view_event',
      panel_id,
      data,
    });
  } else {
    console.warn('[ws] sendViewEvent called but globalSend is null (ws not connected?)');
    return false;
  }
}

export class WsClient {
  private ws: WebSocket | null = null;
  private url: string;
  private onMessage: ((cmd: ViewCommand) => void) | null = null;
  private reconnectTimer: number | null = null;
  private closedByClient = false;
  public status: WsStatus = 'connecting';
  public retry = 0;
  public readonly maxRetries = 5;
  public nextRetryMs = 0;

  constructor(url: string) {
    this.url = url;
  }

  connect(onMessage: (cmd: ViewCommand) => void): void {
    this.onMessage = onMessage;
    this.closedByClient = false;
    window.addEventListener('offline', this.onOffline);
    window.addEventListener('online', this.onOnline);
    this.openSocket();
  }

  private onOffline = () => {
    this.status = 'reconnecting';
    this.ws?.close();
  };

  private onOnline = () => {
    if (this.closedByClient || this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING) return;
    if (this.reconnectTimer !== null) window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    this.openSocket();
  };

  getSnapshot(): WsSnapshot {
    return {
      status: this.status,
      retry: this.retry,
      maxRetries: this.maxRetries,
      nextRetryMs: this.nextRetryMs,
    };
  }

  private openSocket(): void {
    this.ws = new WebSocket(this.url);
    this.status = 'connecting';

    const sendFn = (payload: unknown) => {
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.ws.send(JSON.stringify(payload));
        return true;
      }
      return false;
    };
    setGlobalWs(sendFn);

    this.ws.onopen = () => {
      this.status = 'open';
      this.retry = 0;
      this.nextRetryMs = 0;
      this.send({ type: 'ready', client_id: this.clientId });
    };

    this.ws.onmessage = (ev) => {
      try {
        const cmd = JSON.parse(ev.data) as ViewCommand;
        this.onMessage?.(cmd);
      } catch (e) {
        console.error('ws: bad JSON', e);
      }
    };

    this.ws.onclose = () => {
      setGlobalWs(null);
      this.scheduleReconnect();
    };

    this.ws.onerror = (e) => {
      console.error('ws error', e);
      this.ws?.close();
    };
  }

  send(payload: unknown): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(payload));
    }
  }

  close(): void {
    this.closedByClient = true;
    window.removeEventListener('offline', this.onOffline);
    window.removeEventListener('online', this.onOnline);
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.status = 'closed';
    this.nextRetryMs = 0;
    setGlobalWs(null);
    this.ws?.close();
  }

  private scheduleReconnect(): void {
    if (this.closedByClient) {
      this.status = 'closed';
      return;
    }

    // Keep trying at the capped interval. The view is often opened before the
    // runtime finishes restoring a large checkpoint; giving up after ~23s
    // leaves an otherwise healthy browser permanently empty.
    this.retry = Math.min(this.retry + 1, this.maxRetries);
    this.status = 'reconnecting';
    this.nextRetryMs = Math.min(1000 * 2 ** (this.retry - 1), 8000);

    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer);
    }
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      this.openSocket();
    }, this.nextRetryMs);
  }

  private clientId = `browser-${Math.random().toString(36).slice(2, 10)}`;
}

// Reducer: apply a ViewCommand to a panel map.
export function applyCommand(
  panels: Map<string, PanelState>,
  cmd: ViewCommand
): Map<string, PanelState> {
  // Recursive for batch.
  if (cmd.op === 'batch') {
    let acc = panels;
    for (const c of cmd.commands) {
      acc = applyCommand(acc, c);
    }
    return acc;
  }

  const next = new Map(panels);
  switch (cmd.op) {
    case 'spawn': {
      next.set(cmd.id, {
        id: cmd.id,
        surface: cmd.surface ?? 'spatial3d',
        kind: cmd.kind,
        title: cmd.title,
        position: cmd.position,
        rotation: cmd.rotation,
        scale: [1, 1, 1],
        content: cmd.content,
      });
      break;
    }
    case 'update': {
      const p = next.get(cmd.id);
      if (!p) break;
      if (cmd.patch.patch === 'full') {
        p.content = cmd.patch.content;
      } else {
        // data_model patch — shallow apply at JSON Pointer path.
        applyDataModelPatch(p.content, cmd.patch.path, cmd.patch.value);
      }
      break;
    }
    case 'move': {
      const p = next.get(cmd.id);
      if (!p) break;
      p.position = cmd.position;
      if (cmd.rotation) p.rotation = cmd.rotation;
      break;
    }
    case 'scale': {
      const p = next.get(cmd.id);
      if (!p) break;
      p.scale = cmd.scale;
      break;
    }
    case 'despawn': {
      next.delete(cmd.id);
      break;
    }
    case 'highlight': {
      const p = next.get(cmd.id);
      if (!p) break;
      p.highlighted = cmd.effect;
      p.highlightUntil = Date.now() + cmd.duration_ms;
      break;
    }
  }
  return next;
}

// Minimal JSON Pointer (RFC 6901) patch. Only supports /path/segments on objects/arrays.
function applyDataModelPatch(
  target: unknown,
  path: string,
  value: unknown
): void {
  const parts = path.split('/').filter((s) => s.length > 0);
  let cur: any = target;
  for (let i = 0; i < parts.length - 1; i++) {
    const seg = parts[i];
    if (cur == null) return;
    if (/^\d+$/.test(seg)) {
      cur = cur[Number(seg)];
    } else {
      cur = cur[seg];
    }
  }
  if (cur == null) return;
  const last = parts[parts.length - 1];
  if (/^\d+$/.test(last)) {
    cur[Number(last)] = value;
  } else {
    cur[last] = value;
  }
}
