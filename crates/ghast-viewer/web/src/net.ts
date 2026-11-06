/// <reference lib="dom" />
/**
 * The page's side of the wire: one WebSocket (binary, no socket.io), clock sync with PING/PONG
 * (offset from the lowest-RTT sample), ACKs of the chunk bytes received (the server's flow-control
 * window), byte counters for the overlay, reconnect with backoff.
 */
import { C, S, Reader, messages } from './shared/protocol.js';

export const clientNow = (): number => performance.timeOrigin + performance.now();

export interface NetStats { rtt: number; offset: number; bytesPerSec: number; total: number; chunkBytes: number }

export class Net {
  private ws: WebSocket | null = null;
  private pingTimer = 0;
  private samples: Array<{ at: number; rtt: number; offset: number }> = [];
  /** Offset drift (ms per ms) and the client time the offset line starts at. */
  private drift = 0;
  private offsetAt = 0;
  private chunkBytes = 0;
  private ackedAt = 0;
  private bytesWindow: Array<[number, number]> = [];
  private retry = 500;
  private closedByUs = false;
  /** Server clock − client clock (ms) and the last round trip. */
  offset = 0;
  rtt = 0;
  total = 0;
  connected = false;

  constructor(private readonly url: string, private readonly onMessage: (type: number, r: Reader, frameAt: number) => void,
    private readonly onState: (connected: boolean, why?: string) => void) {}

  /** Estimated server time now (or at a client time). */
  serverNow(at = clientNow()): number { return at + this.offsetFor(at); }
  offsetFor(at: number): number { return this.offset + this.drift * (at - this.offsetAt); }

  connect(): void {
    this.closedByUs = false;
    const ws = new WebSocket(this.url);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    ws.onopen = () => {
      this.connected = true; this.retry = 500;
      this.chunkBytes = 0; this.ackedAt = 0; this.samples = [];
      this.onState(true);
      this.ping();
      clearInterval(this.pingTimer);
      this.pingTimer = window.setInterval(() => this.ping(), 1000);
    };
    ws.onmessage = (e) => this.frame(new Uint8Array(e.data as ArrayBuffer));
    ws.onclose = (e) => {
      this.connected = false;
      clearInterval(this.pingTimer);
      if (this.ws !== ws) return;
      this.ws = null;
      this.onState(false, e.reason || undefined);
      if (!this.closedByUs) { setTimeout(() => this.connect(), this.retry); this.retry = Math.min(10_000, this.retry * 2); }
    };
  }

  close(): void { this.closedByUs = true; this.ws?.close(); }

  send(b: ArrayBuffer): void { if (this.ws?.readyState === 1) this.ws.send(b); }

  private ping(): void {
    const b = new ArrayBuffer(9), v = new DataView(b);
    v.setUint8(0, C.PING); v.setFloat64(1, clientNow(), true);
    this.send(b);
  }

  private frame(buf: Uint8Array): void {
    const at = clientNow();
    this.total += buf.length;
    this.bytesWindow.push([at, buf.length]);
    // pruned here, not only in stats(): that runs only with the debug overlay open, and the list grew for good
    while (at - this.bytesWindow[0][0] > 1000) this.bytesWindow.shift();
    let chunk = false;
    for (const [type, r] of messages(buf)) {
      if (type === S.PONG) { this.pong(r, at); continue; }
      if (type === S.CHUNK) chunk = true;
      this.onMessage(type, r, at);
    }
    if (chunk) {
      this.chunkBytes += buf.length;
      // ACK every 32 KB (and the first column at once): the server keeps at most its window unacknowledged
      if (this.chunkBytes - this.ackedAt >= 32 * 1024 || this.ackedAt === 0) this.ack();
      else setTimeout(() => this.ack(), 30);
    }
  }

  private ack(): void {
    if (this.chunkBytes === this.ackedAt) return;
    this.ackedAt = this.chunkBytes;
    const b = new ArrayBuffer(9), v = new DataView(b);
    v.setUint8(0, C.ACK); v.setFloat64(1, this.chunkBytes, true);
    this.send(b);
  }

  /**
   * Clock offset from the PONGs: the low-RTT samples of the last ~10 s, fitted as a line over time so
   * two clocks that drift apart (a slewing NTP, WSL) still line up; serverNow() extrapolates the line.
   */
  private pong(r: Reader, at: number): void {
    const sent = r.f64(), server = r.f64();
    const rtt = at - sent;
    this.rtt = rtt;
    this.samples.push({ at, rtt, offset: server - (sent + rtt / 2) });
    if (this.samples.length > 10) this.samples.shift();
    let minRtt = Infinity;
    for (const s of this.samples) minRtt = Math.min(minRtt, s.rtt);
    const good = this.samples.filter(s => s.rtt <= minRtt * 1.5 + 2);
    if (good.length >= 3) {
      const t0 = good[0].at;
      let n = 0, sx = 0, sy = 0, sxx = 0, sxy = 0;
      for (const s of good) { const x = s.at - t0; n++; sx += x; sy += s.offset; sxx += x * x; sxy += x * s.offset; }
      const den = n * sxx - sx * sx;
      const slope = den > 0 ? (n * sxy - sx * sy) / den : 0;
      this.drift = Math.abs(slope) < 0.1 ? slope : 0;
      this.offsetAt = t0; this.offset = (sy - this.drift * sx) / n;
    } else {
      this.drift = 0; this.offsetAt = at; this.offset = good[good.length - 1].offset;
    }
  }

  stats(): NetStats {
    const now = clientNow();
    while (this.bytesWindow.length && now - this.bytesWindow[0][0] > 1000) this.bytesWindow.shift();
    let bytes = 0;
    for (const [, n] of this.bytesWindow) bytes += n;
    return { rtt: this.rtt, offset: this.offset, bytesPerSec: bytes, total: this.total, chunkBytes: this.chunkBytes };
  }
}
