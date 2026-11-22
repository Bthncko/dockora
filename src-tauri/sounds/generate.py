"""Dockora bildirim sesleri: sıfırdan sentez (örnek / kayıt yok).

Kısa, yumuşak, yuvarlak tınılar; tık yok (yumuşak atak, sönümlü bitiş), hafif
oda yankısı. Hepsi aynı algılanan yükseklikte, Windows'unkilerden sessiz.

    python generate.py             # bu klasöre <id>.wav yazar (48 kHz, 16 bit, mono)

Gerekenler: numpy, scipy.
"""

import os
import sys
import wave

import numpy as np
from scipy import signal

SR = 48_000
OUT = os.path.dirname(os.path.abspath(__file__))


def timeline(seconds):
    return np.arange(int(SR * seconds)) / SR


def env(t, attack, tau):
    """Yumuşak atak (tık yok) + üstel sönüm; t < 0 sessiz."""
    x = np.maximum(t, 0)
    return np.where(t >= 0, (1 - np.exp(-x / attack)) * np.exp(-x / tau), 0.0)


def glide_phase(t, f_start, f_end, tau):
    """Frekansı f_start'tan f_end'e üstel kayan salınımın fazı (t >= 0 için)."""
    x = np.maximum(t, 0)
    f = f_end + (f_start - f_end) * np.exp(-x / tau)
    return 2 * np.pi * np.cumsum(f) / SR


def at(t, start):
    return t - start


def noise_burst(t, start, length, lo, hi, gain, seed=1):
    """Tokmak / dokunuş hissi: band süzgeçli çok kısa gürültü."""
    rng = np.random.default_rng(seed)
    n = rng.standard_normal(len(t))
    b, a = signal.butter(2, [lo / (SR / 2), hi / (SR / 2)], "band")
    n = signal.lfilter(b, a, n)
    return n * env(at(t, start), 0.0004, length) * gain


def reverb(x, mix, size=1.0):
    """Küçük oda: Schroeder (4 tarak + 2 tüm geçiren), yankı yumuşatılmış."""
    wet = np.zeros_like(x)
    for ms, g in [(29.7, 0.70), (37.1, 0.68), (41.1, 0.66), (43.7, 0.64)]:
        d = int(ms * size * SR / 1000)
        a = np.zeros(d + 1)
        a[0], a[d] = 1, -g
        wet += signal.lfilter([1], a, x)
    for ms, g in [(5.0, 0.7), (1.7, 0.7)]:
        d = int(ms * SR / 1000)
        b = np.zeros(d + 1)
        b[0], b[d] = -g, 1
        a = np.zeros(d + 1)
        a[0], a[d] = 1, -g
        wet = signal.lfilter(b, a, wet)
    b, a = signal.butter(1, 4500 / (SR / 2))
    wet = signal.lfilter(b, a, wet) / 4
    return (1 - mix) * x + mix * wet


def finish(x, fade=0.08):
    """Yumuşatma, sessiz bitiş, ortak yükseklik."""
    b, a = signal.butter(2, 9000 / (SR / 2))
    x = signal.lfilter(b, a, x)
    x = x - np.mean(x)
    n = int(fade * SR)
    x[-n:] *= np.linspace(1, 0, n) ** 2
    x[:16] *= np.linspace(0, 1, 16)
    # Algılanan yükseklik: en gürültülü 100 ms'nin RMS'i → -19 dBFS; tepe en çok -5 dBFS.
    w = int(0.1 * SR)
    rms = max(np.sqrt(np.mean(x[i : i + w] ** 2)) for i in range(0, max(1, len(x) - w), w // 4))
    x *= 10 ** (-19 / 20) / rms
    peak = np.max(np.abs(x))
    if peak > 10 ** (-5 / 20):
        x *= 10 ** (-5 / 20) / peak
    return x


# --- sesler -------------------------------------------------------------------------


def damla():
    """İki küçük kabarcık: yukarı kayan yuvarlak tını."""
    t = timeline(0.55)
    x = np.zeros_like(t)
    for start, f0, f1, tau, gain in [(0.0, 620, 1180, 0.055, 0.85), (0.085, 900, 1560, 0.07, 1.0)]:
        u = at(t, start)
        ph = glide_phase(u, f0, f1, 0.011)
        e = env(u, 0.0025, tau)
        x += gain * e * (np.sin(ph) + 0.12 * np.sin(2 * ph))
    return finish(reverb(x, 0.22))


def marimba(t, start, f, gain):
    u = at(t, start)
    tone = (
        np.sin(2 * np.pi * f * u) * env(u, 0.0015, 0.17)
        + 0.25 * np.sin(2 * np.pi * f * 3.93 * u) * env(u, 0.001, 0.026)
        + 0.05 * np.sin(2 * np.pi * f * 9.2 * u) * env(u, 0.0008, 0.008)
    )
    return gain * tone + noise_burst(t, start, 0.005, 2000, 5000, 0.04 * gain, seed=int(f))


def esinti():
    """Yükselen dörtlü, yumuşak tokmaklı ahşap tını (Sol → Do)."""
    t = timeline(0.8)
    x = marimba(t, 0.0, 783.99, 0.8) + marimba(t, 0.11, 1046.5, 1.0)
    return finish(reverb(x, 0.2))


def cakil():
    """Tek, kısa dokunuş: hafif aşağı kayan yuvarlak 'tok'."""
    t = timeline(0.4)
    ph = glide_phase(t, 1050 * 1.08, 1050, 0.008)
    x = np.sin(ph) * env(t, 0.0012, 0.045)
    x += 0.15 * np.sin(2.7 * ph) * env(t, 0.001, 0.012)
    x += noise_burst(t, 0.0, 0.002, 3500, 9000, 0.06)
    return finish(reverb(x, 0.26), fade=0.1)


def kristal():
    """Tek cam vuruşu: sönümlenen FM parıltı, hafif koro (Mi)."""
    t = timeline(1.05)
    f = 1318.5
    index = 1.6 * np.exp(-t / 0.08)
    x = np.zeros_like(t)
    for detune, gain in [(0.0, 1.0), (0.6, 0.5)]:
        fc = f + detune
        x += gain * np.sin(2 * np.pi * fc * t + index * np.sin(2 * np.pi * 2 * fc * t))
    x *= env(t, 0.003, 0.32)
    x += 0.08 * np.sin(2 * np.pi * 2 * f * t) * env(t, 0.002, 0.1)
    return finish(reverb(x, 0.25), fade=0.15)


def can():
    """Önceki ses (8.5): iki notalı cam/çan, Do → Mi."""
    t = timeline(1.1)

    def bell(freq, start, decay, gain):
        u = np.maximum(at(t, start), 0)
        s = np.zeros_like(t)
        for ratio, amp, dk in [(1.0, 1.0, 1.0), (2.0, 0.22, 0.55), (2.76, 0.10, 0.35), (4.07, 0.035, 0.25)]:
            s += amp * np.sin(2 * np.pi * freq * ratio * u) * np.exp(-u / (decay * dk))
        return np.where(t >= start, s * (1 - np.exp(-u / 0.004)) * gain, 0)

    x = bell(1046.5, 0.0, 0.30, 0.85) + bell(1318.5, 0.085, 0.42, 1.0)
    for d, g in [(0.031, 0.12), (0.047, 0.09), (0.071, 0.06)]:
        n = int(d * SR)
        x[n:] += g * x[:-n]
    return finish(x, fade=0.15)


SOUNDS = {"damla": damla, "esinti": esinti, "cakil": cakil, "kristal": kristal, "can": can}


def write(path, x):
    pcm = np.clip(np.round(x * 32767), -32768, 32767).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else OUT
    for name, make in SOUNDS.items():
        x = make()
        write(os.path.join(out, f"{name}.wav"), x)
        print(f"{name}: {len(x) / SR:.2f} sn, tepe {20 * np.log10(np.max(np.abs(x))):.1f} dBFS")
