"""Original score and sound design for the demo film. No samples, no third-party
music: every sound is synthesized here. Musical events follow the film's own
timeline (output/timeline.json): scene starts, typing intervals, pass/fail
verdicts, transitions.

    python3 score.py  ->  ../output/score.wav (48 kHz stereo)
"""
import json
import wave
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
timeline = json.loads((ROOT / "output/timeline.json").read_text())
DUR = timeline["duration"]
SR = 48000
N = int(DUR * SR) + SR
L = np.zeros(N, np.float32)
R = np.zeros(N, np.float32)
rng = np.random.default_rng(20261007)
BPM = 112.0
BEAT = 60.0 / BPM


def hz(m):
    return 440.0 * 2 ** ((m - 69) / 12)


def add(t, sig, pan=0.0, gain=1.0):
    i = int(t * SR)
    if i >= N or i + len(sig) <= 0:
        return
    if i < 0:
        sig = sig[-i:]
        i = 0
    n = min(len(sig), N - i)
    l = np.sqrt((1 - pan) / 2) * gain
    r = np.sqrt((1 + pan) / 2) * gain
    L[i:i + n] += sig[:n] * l
    R[i:i + n] += sig[:n] * r


def env(n, a, d, s=1.0, rel=None):
    t = np.arange(n) / SR
    e = np.minimum(1, t / max(a, 1e-4))
    if rel:
        e *= np.exp(-np.maximum(0, t - d) / rel) if s >= 1 else 1
    else:
        e *= np.exp(-t / d)
    return e


def lowpass(x, cutoff):
    # one-pole low-pass, cheap and smooth
    a = np.exp(-2 * np.pi * cutoff / SR)
    y = np.empty_like(x)
    acc = 0.0
    b = 1 - a
    for i in range(len(x)):
        acc = b * x[i] + a * acc
        y[i] = acc
    return y


def lp_fast(x, cutoff):
    # FFT-domain low-pass for long buffers
    X = np.fft.rfft(x)
    f = np.fft.rfftfreq(len(x), 1 / SR)
    X *= 1 / np.sqrt(1 + (f / cutoff) ** 4)
    return np.fft.irfft(X, len(x)).astype(np.float32)


scenes = timeline["scenes"]
cues = timeline["cues"]
start_of = {s["id"]: s["at"] for s in scenes}
GROOVE_IN = start_of["who"]
BREAK_IN = start_of["boundaries"]
END_IN = start_of["end"]
SOFT_IN = 4.4

# ── harmony: Dm9 – Bbmaj7 – F/A – Csus, two bars each ───────────────────────
chords = [[50, 57, 60, 64, 65], [46, 53, 57, 62, 65], [45, 53, 57, 60, 64], [48, 55, 60, 62, 67]]
bar = 4 * BEAT
progression_len = 2 * bar

# Pad: detuned additive saws, slow attack, filtered.
pad = np.zeros(N, np.float32)
t0 = 0.0
k = 0
while t0 < DUR + 2:
    notes = chords[k % 4]
    n = int((progression_len + 1.5) * SR)
    t = np.arange(n) / SR
    e = np.minimum(1, t / 1.2) * np.minimum(1, np.maximum(0, (progression_len + 1.5 - t)) / 1.4)
    sig = np.zeros(n, np.float32)
    for m in notes:
        for det in (-0.07, 0.0, 0.08):
            f = hz(m + det)
            for h in range(1, 7):
                sig += (np.sin(2 * np.pi * f * h * t + h * 0.3) / h) * (0.9 ** h)
    sig *= e * 0.012
    i = int(t0 * SR)
    m_ = min(n, N - i)
    if m_ > 0:
        pad[i:i + m_] += sig[:m_]
    t0 += progression_len
    k += 1
pad = lp_fast(pad, 1800)
# pad swells in from silence; a touch lower under dense terminal scenes
pad_gain = np.clip(np.arange(N) / SR / 4.0, 0, 1)
L += pad * pad_gain * 1.7
R += pad * pad_gain * 1.7

# ── rhythm section ─────────────────────────────────────────────────────────
def kick(t, g=1.0):
    n = int(0.42 * SR)
    tt = np.arange(n) / SR
    f = 46 + 110 * np.exp(-tt * 32)
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-tt * 9)
    s += 0.25 * np.sin(2 * np.pi * np.cumsum(f * 2) / SR) * np.exp(-tt * 40)
    add(t, s.astype(np.float32), 0, 0.30 * g)


def hat(t, g=1.0, open_=False):
    n = int((0.18 if open_ else 0.05) * SR)
    x = rng.normal(0, 1, n).astype(np.float32)
    x = np.diff(x, prepend=0)  # crude high-pass
    x *= np.exp(-np.arange(n) / SR * (18 if open_ else 70))
    add(t, x, 0.25, 0.028 * g)


def clap(t, g=1.0):
    n = int(0.22 * SR)
    x = rng.normal(0, 1, n).astype(np.float32)
    e = np.zeros(n)
    for o in (0, 0.011, 0.022):
        i = int(o * SR)
        e[i:] += np.exp(-np.arange(n - i) / SR * 30)
    x = np.diff(x, prepend=0) * e
    add(t, x.astype(np.float32), -0.1, 0.05 * g)


duck = np.ones(N, np.float32)  # sidechain envelope for bass and pad
beat_t = 0.0
b = 0
while beat_t < DUR:
    in_groove = GROOVE_IN - 0.05 <= beat_t < BREAK_IN
    soft = SOFT_IN <= beat_t < GROOVE_IN
    outro = beat_t >= END_IN
    if in_groove or (soft and b % 2 == 0):
        kick(beat_t, 0.75 if soft else 1.0)
        i = int(beat_t * SR)
        n = int(0.25 * SR)
        m_ = min(n, N - i)
        duck[i:i + m_] = np.minimum(duck[i:i + m_], 1 - 0.55 * np.exp(-np.arange(m_) / SR * 14))
    if in_groove:
        for s16 in range(4):
            hat(beat_t + s16 * BEAT / 4, 1.0 if s16 == 2 else 0.55, open_=(s16 == 2 and b % 4 == 3))
        if b % 2 == 1 and beat_t > start_of["website"]:
            clap(beat_t)
    if outro and b % 4 == 0 and beat_t < END_IN + 1:
        kick(beat_t, 1.0)
    beat_t += BEAT
    b += 1

# Bass: 8th-note pulse on the chord root, ducked by the kick.
bass = np.zeros(N, np.float32)
t = GROOVE_IN
while t < BREAK_IN:
    idx = int((t // progression_len)) % 4
    root = chords[idx][0] - 12
    n = int(BEAT / 2 * SR)
    tt = np.arange(n) / SR
    f = hz(root)
    s = (np.sin(2 * np.pi * f * tt) + 0.3 * np.sin(2 * np.pi * 2 * f * tt)) * np.minimum(1, tt / 0.006) * np.exp(-tt * 5)
    i = int(t * SR)
    m_ = min(n, N - i)
    bass[i:i + m_] += s[:m_].astype(np.float32) * 0.10
    t += BEAT / 2
bass *= duck
L += bass
R += bass

# Arpeggio: 16th-note plucks over chord tones, two octaves up, ping-pong.
pattern = [0, 2, 4, 3, 1, 4, 2, 3]
t = SOFT_IN
step = 0
while t < END_IN + 2:
    idx = int((t // progression_len)) % 4
    notes = chords[idx]
    m = notes[pattern[step % 8] % len(notes)] + 12 + (12 if step % 16 >= 12 else 0)
    n = int(0.5 * SR)
    tt = np.arange(n) / SR
    f = hz(m)
    s = (np.sin(2 * np.pi * f * tt) + 0.22 * np.sin(2 * np.pi * 2 * f * tt) + 0.08 * np.sin(2 * np.pi * 3 * f * tt)) * np.minimum(1, tt / 0.003) * np.exp(-tt * 9)
    g = 0.036 if t < GROOVE_IN else (0.05 if t < BREAK_IN else 0.042)
    add(t, s.astype(np.float32), -0.55 if step % 2 else 0.55, g)
    add(t + BEAT * 0.75, s.astype(np.float32) * 0.35, 0.55 if step % 2 else -0.55, g)  # dotted echo
    t += BEAT / 4
    step += 1

L *= np.where(np.arange(N) / SR > GROOVE_IN, duck * 0.25 + 0.75, 1)
R *= np.where(np.arange(N) / SR > GROOVE_IN, duck * 0.25 + 0.75, 1)

# ── sound design from the film's cues ───────────────────────────────────────
def whoosh(t, g=1.0, length=0.9):
    n = int(length * SR)
    x = rng.normal(0, 1, n).astype(np.float32)
    tt = np.arange(n) / SR
    # sweep a resonant band upwards, then let it fall
    x = lp_fast(x, 2500) * np.sin(np.pi * np.clip(tt / length, 0, 1)) ** 2
    add(t - length * 0.55, x, -0.3, 0.05 * g)
    add(t - length * 0.5, x[::-1].copy(), 0.3, 0.035 * g)


def boom(t, g=1.0):
    n = int(2.5 * SR)
    tt = np.arange(n) / SR
    f = 38 + 60 * np.exp(-tt * 6)
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-tt * 1.6)
    noise = lp_fast(rng.normal(0, 1, n).astype(np.float32), 400) * np.exp(-tt * 3)
    add(t, (s * 0.6 + noise * 0.4).astype(np.float32), 0, 0.5 * g)
    # shimmer
    for m in (74, 81, 86):
        sh = np.sin(2 * np.pi * hz(m) * tt) * np.exp(-tt * 1.2) * np.minimum(1, tt / 0.02)
        add(t, sh.astype(np.float32), rng.uniform(-.6, .6), 0.012 * g)


def tick(t, g=1.0):
    n = int(0.06 * SR)
    tt = np.arange(n) / SR
    s = np.sin(2 * np.pi * 2400 * tt) * np.exp(-tt * 90)
    add(t, s.astype(np.float32), rng.uniform(-.4, .4), 0.03 * g)


def keyclick(t, g=1.0):
    n = int(0.03 * SR)
    x = rng.normal(0, 1, n).astype(np.float32)
    x = np.diff(x, prepend=0) * np.exp(-np.arange(n) / SR * 260)
    tt = np.arange(n) / SR
    x += (np.sin(2 * np.pi * rng.uniform(900, 1300) * tt) * np.exp(-tt * 300)).astype(np.float32) * 0.5
    add(t, x, rng.uniform(-.25, .25), 0.022 * g)


def chime(t, up=True, g=1.0):
    seq = (76, 83) if up else (64, 63)
    for j, m in enumerate(seq):
        n = int(1.2 * SR)
        tt = np.arange(n) / SR
        f = hz(m)
        s = (np.sin(2 * np.pi * f * tt) + 0.3 * np.sin(2 * np.pi * 2 * f * tt)) * np.exp(-tt * 3.5) * np.minimum(1, tt / 0.004)
        add(t + j * 0.09, s.astype(np.float32), 0.15 if j else -0.15, 0.06 * g)


def thud(t, g=1.0):
    n = int(0.8 * SR)
    tt = np.arange(n) / SR
    f = 70 * np.exp(-tt * 3) + 45
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-tt * 6)
    add(t, s.astype(np.float32), 0, 0.32 * g)
    chime(t + 0.02, up=False, g=0.7)


for c in cues:
    t, ty = c["t"], c["type"]
    if ty == "scene" and t > 0.5:
        whoosh(t, 0.8)
    elif ty == "whoosh":
        whoosh(t, 1.0)
    elif ty == "boom":
        boom(t)
    elif ty == "tick":
        tick(t)
    elif ty == "pass":
        chime(t, True)
    elif ty == "fail":
        thud(t)
    elif ty == "typing":
        k = 0
        tt = t
        while tt < t + c["dur"]:
            keyclick(tt, 0.9 if k % 5 else 1.15)
            tt += rng.uniform(0.045, 0.085)
            k += 1



# ── master ─────────────────────────────────────────────────────────────────
mix = np.stack([L[: int(DUR * SR)], R[: int(DUR * SR)]], 1)
tt = np.arange(len(mix)) / SR
fade = np.minimum(1, tt / 0.6) * np.minimum(1, np.maximum(0, DUR - tt) / 2.5)
mix *= fade[:, None]
# loudness: aim near -16 LUFS-ish by RMS, then soft-clip
rms = np.sqrt(np.mean(mix ** 2))
mix *= (10 ** (-18 / 20)) / max(rms, 1e-9)
mix = np.tanh(mix * 1.2) / np.tanh(1.2)
peak = np.max(np.abs(mix))
mix *= 0.89 / peak
out = ROOT / "output/score.wav"
with wave.open(str(out), "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((mix * 32767).astype("<i2").tobytes())
print(f"wrote {out} ({DUR:.1f}s)")
