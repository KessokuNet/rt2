import struct
import sys

from elftools.elf.elffile import ELFFile

path, start = sys.argv[1], int(sys.argv[2], 16)
f = open(path, "rb")
elf = ELFFile(f)


def off(va):
    for seg in elf.iter_segments():
        if seg["p_type"] == "PT_LOAD" and seg["p_vaddr"] <= va < seg["p_vaddr"] + seg["p_filesz"]:
            return va - seg["p_vaddr"] + seg["p_offset"]
    raise ValueError(hex(va))


def cstr(va):
    f.seek(off(va))
    return f.read(256).split(b"\0")[0].decode()


i = 0
while True:
    f.seek(off(start + i * 24))
    cls, name, sig = struct.unpack("<3I", f.read(12))
    if not cls:
        break
    print(f"{i:3d} {cstr(cls)}.{cstr(name)} {cstr(sig)}")
    i += 1
