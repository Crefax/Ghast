/// <reference lib="dom" />
/**
 * Recordings on the page: the Recordings chip lists them (GET replays), picking one reloads the page with
 * ?replay=<name>; in a recording the bar at the bottom plays, pauses, seeks and changes the speed (C.REPLAY to
 * the server, which answers with S.REPLAY a few times a second).
 */
import { C } from './shared/protocol.js';

interface Listed { name: string; start: number; end: number | null; bytes: number; note?: string }

const SPEEDS = [1, 2, 4, 8, 16];
const pad = (n: number) => String(n).padStart(2, '0');
const clock = (ms: number) => { const d = new Date(ms); return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`; };

/** The page's URL with these query parameters set (null: removed). */
function withParams(set: Record<string, string | null>): string {
  const q = new URLSearchParams(location.search);
  for (const [k, v] of Object.entries(set)) { if (v === null) q.delete(k); else q.set(k, v); }
  return `${location.pathname}?${q}`;
}

/** The C.REPLAY message: f64 speed (0 paused, NaN unchanged), f64 seek ms from the start (−1: none). */
export function replayControl(speed: number, seek: number): ArrayBuffer {
  const b = new ArrayBuffer(17), v = new DataView(b);
  v.setUint8(0, C.REPLAY); v.setFloat64(1, speed, true); v.setFloat64(9, seek, true);
  return b;
}

/** "m:ss" of a span in ms. */
export const span = (ms: number) => { const s = Math.max(0, Math.round(ms / 1000)); return `${Math.floor(s / 60)}:${pad(s % 60)}`; };

export class ReplayUi {
  private bar: HTMLElement | null = null;
  private slider: HTMLInputElement | null = null;
  private label: HTMLElement | null = null;
  private playBtn: HTMLButtonElement | null = null;
  private speedBtns: HTMLButtonElement[] = [];
  private dragging = false;
  private state = { cursor: 0, duration: 0, speed: 1, start: 0 };
  private lastSpeed = 1;

  constructor(private readonly send: (b: ArrayBuffer) => void, readonly name: string | null) {
    const chip = document.createElement('button');
    chip.id = 'rec';
    chip.title = name ? 'Back to the live view' : 'Recordings of this bot';
    chip.textContent = name ? '● Live' : '◉ Recordings';
    chip.addEventListener('click', () => { if (name) location.href = withParams({ replay: null, t: null, speed: null }); else void this.toggleList(chip); });
    document.getElementById('modes')?.insertBefore(chip, document.getElementById('dbg'));
    if (name) this.buildBar();
  }

  private async toggleList(chip: HTMLElement): Promise<void> {
    const old = document.getElementById('rec-list');
    if (old) { old.remove(); return; }
    const box = document.createElement('div');
    box.id = 'rec-list';
    box.textContent = 'Loading…';
    document.body.appendChild(box);
    const r = chip.getBoundingClientRect();
    box.style.top = `${r.bottom + 6}px`;
    try {
      const list = await (await fetch('replays', { cache: 'no-store' })).json() as Listed[];
      box.textContent = '';
      if (!list.length) { box.textContent = 'No recordings yet. The bot application decides when to record (see the README).'; return; }
      for (const m of list) {
        const a = document.createElement('a');
        const d = new Date(m.start);
        a.href = withParams({ replay: m.name, t: null, speed: null });
        a.innerHTML = `<b>${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${clock(m.start).slice(0, 5)}</b> – ${m.end ? clock(m.end).slice(0, 5) : 'recording'}`
          + ` <span>${m.end ? span(m.end - m.start) : ''} · ${(m.bytes / 1048576).toFixed(1)} MB</span>`;
        if (m.note) { const n = document.createElement('i'); n.textContent = ' ' + m.note; a.appendChild(n); }
        box.appendChild(a);
      }
    } catch { box.textContent = 'Could not load the recordings'; }
  }

  private buildBar(): void {
    const bar = document.createElement('div');
    bar.id = 'rec-bar';
    bar.innerHTML = `<button class="play" title="Play / pause">❚❚</button><input type="range" min="0" max="1000" value="0">`
      + `<span class="time">–</span><span class="speeds">${SPEEDS.map(s => `<button data-speed="${s}">${s}×</button>`).join('')}</span>`;
    document.body.appendChild(bar);
    document.body.classList.add('replay');
    this.bar = bar;
    this.slider = bar.querySelector('input');
    this.label = bar.querySelector('.time');
    this.playBtn = bar.querySelector('.play');
    this.speedBtns = Array.from(bar.querySelectorAll<HTMLButtonElement>('[data-speed]'));
    this.playBtn!.addEventListener('click', () => {
      if (this.state.speed > 0) { this.lastSpeed = this.state.speed; this.control(0, -1); }
      else this.control(this.lastSpeed, this.state.cursor >= this.state.duration - 500 ? 0 : -1); // at the end: from the start
    });
    for (const b of this.speedBtns) b.addEventListener('click', () => { this.lastSpeed = Number(b.dataset.speed); this.control(this.lastSpeed, -1); });
    const s = this.slider!;
    s.addEventListener('input', () => { this.dragging = true; this.paint((Number(s.value) / 1000) * this.state.duration); });
    s.addEventListener('change', () => { this.dragging = false; this.control(Number.NaN, (Number(s.value) / 1000) * this.state.duration); });
  }

  /** speed NaN: unchanged; seek −1: none. */
  private control(speed: number, seek: number): void { this.send(replayControl(speed, seek)); }

  /** S.REPLAY from the server. */
  update(cursor: number, duration: number, speed: number, start: number): void {
    this.state = { cursor, duration, speed, start };
    if (speed > 0) this.lastSpeed = speed;
    if (!this.bar) return;
    this.playBtn!.textContent = speed > 0 ? '❚❚' : '▶';
    for (const b of this.speedBtns) b.classList.toggle('on', Number(b.dataset.speed) === (speed || this.lastSpeed));
    if (!this.dragging) { this.slider!.value = String(duration > 0 ? Math.round((cursor / duration) * 1000) : 0); this.paint(cursor); }
  }

  private paint(cursor: number): void {
    const { duration, start } = this.state;
    this.label!.textContent = `${clock(start + cursor)} · ${span(cursor)} / ${span(duration)}`;
  }
}
