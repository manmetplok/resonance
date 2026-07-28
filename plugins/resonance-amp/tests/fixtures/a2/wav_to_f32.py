#!/usr/bin/env python3
"""Extract the data chunk of an IEEE float32 mono WAV as raw LE f32."""
import struct, sys

def extract(path):
    b = open(path, 'rb').read()
    assert b[:4] == b'RIFF' and b[8:12] == b'WAVE', path
    pos = 12
    fmt = None
    while pos + 8 <= len(b):
        cid, size = b[pos:pos+4], struct.unpack('<I', b[pos+4:pos+8])[0]
        body = b[pos+8:pos+8+size]
        if cid == b'fmt ':
            fmt = struct.unpack('<HHIIHH', body[:16])
        elif cid == b'data':
            assert fmt and fmt[0] == 3 and fmt[1] == 1 and fmt[5] == 32, (path, fmt)
            return body
        pos += 8 + size + (size & 1)
    raise ValueError('no data chunk: ' + path)

if __name__ == '__main__':
    for p in sys.argv[1:]:
        out = p.rsplit('.', 1)[0] + '.f32'
        open(out, 'wb').write(extract(p))
        print(out)
