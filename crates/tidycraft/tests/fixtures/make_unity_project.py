"""Hand-built fixture assets. Every file is a real format the analyzers parse;
what each one violates is decided from docs/analyzer-rules.md, not from running
the tool. Unity then imports the tree and writes the .meta sidecars."""
import random
import struct
import wave
import zlib
from pathlib import Path

A = Path("unity-project/Assets")
NOISE = random.Random(1)  # incompressible pixels, but the same bytes on every run


def png(path, w, h, rgba, srgb=False, noise=False):
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    rows = b"".join(b"\x00" + (NOISE.randbytes(4 * w) if noise else bytes(rgba) * w) for _ in range(h))
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    if srgb:
        out += chunk(b"sRGB", b"\x00")
    out += chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(out)
    return len(out)


def obj(path, verts, faces, mats=None):
    # Materials come from a side-loaded .mtl, as every real OBJ export has one.
    lines = [f"mtllib {path.stem}.mtl"] if mats else []
    lines += [f"v {x} {y} {z}" for x, y, z in verts]
    for i, f in enumerate(faces):
        if mats and i in mats:
            lines.append(f"usemtl {mats[i]}")
        lines.append("f " + " ".join(str(k) for k in f))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", newline="\n")
    if mats:
        mtl = "".join(f"newmtl {m}\nKd 0.5 0.5 0.5\n" for m in dict.fromkeys(mats.values()))
        path.with_suffix(".mtl").write_text(mtl, newline="\n")


def wav(path, rate, channels, seconds, fill):
    # Distinct fill bytes per file so no two clips share content (duplicate is always on).
    path.parent.mkdir(parents=True, exist_ok=True)
    n = int(rate * seconds)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(bytes([fill]) * (2 * channels * n))
    return path.stat().st_size


T = A / "Textures"
# clean set member (basecolor without normal -> pbr_set.incomplete anchors here)
png(T / "T_Hero_BaseColor.png", 8, 8, (200, 40, 40, 255))
# complete set; the normal map is tagged sRGB -> texture.color_space
png(T / "T_Rock_BaseColor.png", 8, 8, (90, 90, 90, 255))
png(T / "T_Rock_normal.png", 8, 8, (128, 128, 255, 255), srgb=True)
png(T / "T_NonPot.png", 6, 6, (10, 200, 10, 255))      # texture.pot
png(T / "T_Tiny.png", 2, 2, (10, 10, 200, 255))        # texture.min_size (< 4)
png(T / "T_Huge.png", 32, 32, (200, 200, 10, 255))     # texture.max_size (> 16)
png(T / "T_Wide.png", 8, 4, (200, 10, 200, 255))       # texture.non_square
heavy = png(T / "T_Heavy.png", 16, 16, None, noise=True)  # texture.file_size (> 1000 B)
png(T / "Rock.png", 8, 8, (1, 2, 3, 255))               # naming.prefix (no T_)
png(T / "T_Bad Name.png", 8, 8, (4, 5, 6, 255))         # naming.forbidden_char
png(T / "T_中文.png", 8, 8, (7, 8, 9, 255))              # naming.chinese
png(T / "T_AVeryLongTextureNameThatExceedsTheLimit.png", 8, 8, (11, 12, 13, 255))  # naming.length
png(T / "T_Dup_A.png", 8, 8, (50, 60, 70, 255))         # duplicate pair
png(T / "T_Dup_B.png", 8, 8, (50, 60, 70, 255))
(T / "sources").mkdir(exist_ok=True)
(T / "sources" / "T_Rock_BaseColor.spp").write_bytes(b"substance painter project stand-in\n")  # dcc_source (mtime set by the test)

M = A / "Models"
cube_v = [(x, y, z) for x in (0, 1) for y in (0, 1) for z in (0, 1)]
cube_f = [(1, 2, 4), (1, 4, 3), (5, 7, 8), (5, 8, 6), (1, 5, 6), (1, 6, 2),
          (3, 4, 8), (3, 8, 7), (1, 3, 7), (1, 7, 5), (2, 6, 8), (2, 8, 4)]
obj(M / "SM_Cube.obj", cube_v, cube_f)                                   # clean: 8 v / 12 f
dense_v = [(i, 0, 0) for i in range(12)]
dense_f = [(i + 1, i + 2, i + 3) for i in range(10)]
obj(M / "SM_Dense.obj", dense_v, dense_f)                                # model.vertices (12 > 10)
obj(M / "SM_ManyFaces.obj", cube_v, cube_f + [(1, 2, 8), (3, 4, 5)])    # model.faces (14 > 12)
obj(M / "SM_TwoMats.obj", [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0)], [(1, 2, 3), (2, 4, 3)],
    mats={0: "MatA", 1: "MatB"})                                         # model.materials (2 > 1)

S = A / "Audio"
print("SFX_Click", wav(S / "SFX_Click.wav", 44100, 1, 0.1, 1))       # clean
print("SFX_LowRate", wav(S / "SFX_LowRate.wav", 22050, 1, 0.1, 2))   # audio.sample_rate
print("SFX_Long", wav(S / "SFX_Long.wav", 44100, 1, 1.0, 3))         # audio.sfx_duration (> 0.5 s)
print("SFX_Stereo", wav(S / "SFX_Stereo.wav", 44100, 2, 0.1, 4))     # audio.stereo_sfx
print("Music_Loop", wav(S / "Music_Loop.wav", 44100, 2, 0.5, 5))     # clean: no sfx keyword
print("Ambient_Big", wav(S / "Ambient_Big.wav", 44100, 2, 1.0, 6))   # audio.file_size (> 150000 B)
print("T_Heavy bytes", heavy)
