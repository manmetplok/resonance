#!/usr/bin/env python3
"""Deterministic test input for NAM A2 reference-parity fixtures.

4096 mono samples @ 48000 Hz, float32:
  n == 0          : 1.0 (unit impulse)
  1 <= n < 1024   : 0.0 (decay gap)
  1024 <= n < 4096: 0.5 * sin(2*pi*220*(n-1024)/48000), computed in
                    IEEE-754 double, then cast to float32.

Writes input.f32 (raw little-endian float32) and input.wav
(48 kHz mono IEEE float32 WAV).
"""
import math, struct, sys

N, SR, F, AMP, START = 4096, 48000, 220.0, 0.5, 1024
samples = []
for n in range(N):
    if n == 0:
        v = 1.0
    elif n < START:
        v = 0.0
    else:
        v = AMP * math.sin(2.0 * math.pi * F * (n - START) / SR)
    samples.append(struct.unpack('<f', struct.pack('<f', v))[0])

raw = b''.join(struct.pack('<f', v) for v in samples)
open('input.f32', 'wb').write(raw)

data_size = len(raw)
with open('input.wav', 'wb') as w:
    w.write(b'RIFF' + struct.pack('<I', 36 + data_size) + b'WAVE')
    w.write(b'fmt ' + struct.pack('<IHHIIHH', 16, 3, 1, SR, SR * 4, 4, 32))
    w.write(b'data' + struct.pack('<I', data_size) + raw)
print(f'wrote input.f32/input.wav ({N} samples @ {SR} Hz)')
