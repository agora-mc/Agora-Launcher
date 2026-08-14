/**
 * Workshop feedback sounds — small WebAudio blips that respect the global
 * ambience sound setting (read from localStorage; the Lab cannot import
 * Tauri, so it never calls getSetting directly).
 */

let soundOn = false;

export function syncWorkshopSound(): void {
  try {
    const raw = window.localStorage.getItem('ambience.sound');
    soundOn = raw === 'true' || raw === '1';
  } catch {
    soundOn = false;
  }
}

let ac: AudioContext | null = null;

function ctx(): AudioContext | null {
  try {
    const Ctor = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    return Ctor ? new Ctor() : null;
  } catch {
    return null;
  }
}

export function blip(f: number, d = 0.12, type: OscillatorType = 'triangle', v = 0.07): void {
  if (!soundOn) return;
  try {
    ac = ac || ctx();
    if (!ac) return;
    const o = ac.createOscillator(), g = ac.createGain();
    o.type = type; o.frequency.value = f;
    g.gain.setValueAtTime(0.0001, ac.currentTime);
    g.gain.exponentialRampToValueAtTime(v, ac.currentTime + 0.012);
    g.gain.exponentialRampToValueAtTime(0.0001, ac.currentTime + d);
    o.connect(g); g.connect(ac.destination);
    o.start(); o.stop(ac.currentTime + d + 0.03);
  } catch {
    // never let sound break the lesson
  }
}

export function chord(list: number[], gap = 70): void {
  list.forEach((f, i) => setTimeout(() => blip(f, 0.16), i * gap));
}

export function yay(): void { chord([523, 659, 784, 1047], 70); }
export function nope(): void { blip(180, 0.22, 'square', 0.06); }
