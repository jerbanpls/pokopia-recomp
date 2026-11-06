#!/usr/bin/env python3
"""Build the Ampere (NVN2) shader corpus of a KTGL game (Pokemon Pokopia) from its RDB data and main.

Sources:
  * romfs motor_asset_re/<platform>/*.fdata and data/*.file: IDRK entries (see .weaver/ktgl_rdb.py); entries whose
    inflated data starts with '_S2G' (G2S shader container, type KTID 0x7bcd279f) or '_DOK' (system resources,
    type KTID 0x20a6a0bb) are searched for GLSLC outputs.
  * main (decompressed ELF/NSO segment dump, or the ExeFS 'main' NSO itself, decompressed here): searched for GLSLC
    outputs as is.

A GLSLC output (NVN2 layout, see docs/AMPERE.md): u32 magic 0x19866891, +0x5c u32 output length, +0x64 u32 section count,
section header i at +0xa8 + 0x98*i = {u32 size, u32 offset from output, u32 type}; type 0 (GPU code) adds
+0x30 control offset, +0x34 code offset (both from the section data), +0x38 code length, +0x3c control length.

Output (--out), content-addressed by the first 16 hex of SHA-1 of the bytes: code/<h>.bin (code area: SPH v4, then
SASS), ctrl/<h>.bin (control sections), glslc/<h>.bin (whole GLSLC outputs); manifest.tsv (one row per occurrence with
all three hashes), unique.tsv (one row per distinct code block, with its first occurrence's ctrl/glslc), summary.txt.
Deterministic: files and entries are visited in sorted order; "first occurrence" is in that order.
Usage: python -I extract_corpus.py --romfs <romfs> [--romfs-base <base romfs>] [--main <main.elf>] --out <dir> [--jobs 4]
  --romfs-base: the base game's romfs under an update's (the update's files win), for an update dumped without the base game's files
"""
import argparse, collections, hashlib, mmap, os, struct, sys, zlib
from concurrent.futures import ProcessPoolExecutor

IDRK = b'IDRK0000'
GLSLC_MAGIC = b'\x91\x68\x86\x19'
CTRL_MAGIC = 0x98761234
CTRL_STAGES = {0: 'vs', 1: 'ps', 2: 'gs', 3: 'tcs', 4: 'tes', 5: 'cs'}


def low_priority():
    if os.name == 'nt':
        import ctypes
        ctypes.windll.kernel32.SetPriorityClass(ctypes.windll.kernel32.GetCurrentProcess(), 0x4000)  # BELOW_NORMAL


def entries(d):
    o, n = 0, len(d)
    while o + 0x30 <= n:
        if d[o:o + 8] != IDRK:
            o = d.find(IDRK, o)
            if o < 0:
                return
        esz, csz, usz = struct.unpack_from('<QQQ', d, o + 8)
        _, fk, tk, fl = struct.unpack_from('<4I', d, o + 0x20)
        if esz < 0x30 or o + esz > n:
            return
        yield o, o + esz - csz, o + esz, usz, fk, tk, fl
        o += esz


def inflate(d, a, b, flags, head=None):
    if not flags & 0x500000:
        return bytes(d[a:min(b, a + head) if head else b])
    new, lz4b = flags & 0x400000, (flags & 0x500000) == 0x500000
    out, got = [], 0
    while a + 4 <= b:
        if new:
            cs = struct.unpack_from('<H', d, a)[0]; a += 10
        else:
            cs = struct.unpack_from('<I', d, a)[0]; a += 4
        if not cs:
            break
        if lz4b:
            import lz4.block
            r = lz4.block.decompress(bytes(d[a:a + cs]), uncompressed_size=0x4000)
        else:
            r = zlib.decompress(bytes(d[a:a + cs]))
        out.append(r); got += len(r); a += cs
        if head and got >= head:
            break
    r = b''.join(out)
    return r[:head] if head else r


def stage_of(ctrl):
    """NVN stage from control +0x56 (u16): 0 vs, 1 ps, 2 gs, 3 tcs, 4 tes, 5 cs. Agrees with the SPH v4 shader type of
    every graphics block; one compute block's first instruction word happens to look like an SPH, so the SPH is not used."""
    return CTRL_STAGES.get(struct.unpack_from('<H', ctrl, 0x56)[0], '?') if len(ctrl) >= 0x58 else '?'


def glslc_outputs(d):
    """-> [(output offset, output bytes, [(section index, ctrl, code)])]"""
    res = []
    o = d.find(GLSLC_MAGIC)
    while o >= 0:
        ok = False
        if o + 0xa8 <= len(d):
            size, = struct.unpack_from('<I', d, o + 0x5c)
            nsec, = struct.unpack_from('<I', d, o + 0x64)
            if 0xa8 < size <= len(d) - o and 0 < nsec <= 16 and 0xa8 + 0x98 * nsec <= size:
                secs = []
                for i in range(nsec):
                    s = o + 0xa8 + 0x98 * i
                    ssz, soff, typ = struct.unpack_from('<3I', d, s)
                    if typ != 0:
                        continue
                    co, ko, kn, cn = struct.unpack_from('<4I', d, s + 0x30)
                    if soff + ssz > size or co + cn > ssz or ko + kn > ssz:
                        secs = None; break
                    base = o + soff
                    ctrl, code = bytes(d[base + co:base + co + cn]), bytes(d[base + ko:base + ko + kn])
                    if len(ctrl) < 8 or struct.unpack_from('<I', ctrl)[0] != CTRL_MAGIC:
                        secs = None; break
                    secs.append((i, ctrl, code))
                if secs:
                    res.append((o, bytes(d[o:o + size]), secs))
                    ok = True
                    o = d.find(GLSLC_MAGIC, o + size)
                    continue
        o = d.find(GLSLC_MAGIC, o + 4)
    return res


OUT = None


def put(sub, h, data):
    """Content-addressed write (same name = same bytes, so parallel workers cannot disagree)."""
    p = os.path.join(OUT, sub, h + '.bin')
    if not os.path.exists(p):
        t = f'{p}.{os.getpid()}.tmp'
        with open(t, 'wb') as f:
            f.write(data)
        try:
            os.replace(t, p)
        except PermissionError:
            # Windows refuses to replace a file another worker (or a virus scanner) has open. That worker wrote the same
            # block (content-addressed), so keep its copy.
            if not os.path.exists(p):
                raise
            os.remove(t)


def slim(occs):
    """Write the blobs; return the rows without bytes."""
    rows = []
    for name, eo, fk, tk, tag, go, si, st, ctrl, code, gb in occs:
        h = hashlib.sha1(code).hexdigest()[:16]
        ch = hashlib.sha1(ctrl).hexdigest()[:16]
        gh = hashlib.sha1(gb).hexdigest()[:16]
        put('code', h, code); put('ctrl', ch, ctrl); put('glslc', gh, gb)
        rows.append((name, eo, fk, tk, tag, go, si, st, len(ctrl), len(code), h, ch, gh))
    return rows


def scan_file(path, out):
    global OUT
    OUT = out
    low_priority()
    rows = []
    name = os.path.basename(path)
    with open(path, 'rb') as f, mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ) as d:
        for o, a, b, usz, fk, tk, fl in entries(d):
            tag = inflate(d, a, b, fl, 8)
            if tag[:4] not in (b'_S2G', b'_DOK'):
                continue
            data = inflate(d, a, b, fl)
            rows += slim([(name, o, fk, tk, tag.decode('latin1'), go, si, stage_of(ctrl), ctrl, code, gb)
                          for go, gb, secs in glslc_outputs(data) for si, ctrl, code in secs])
    return rows


def nso_image(d):
    """A compressed NSO (the ExeFS 'main' as dumped) -> its memory image: .text, .rodata, .data at their memory offsets
    (header +0x10 three {file off, mem off, size, align}, +0x60 compressed sizes, flag bits 0..2 = segment LZ4-compressed)."""
    flags, = struct.unpack_from('<I', d, 0xc)
    segs = [struct.unpack_from('<4I', d, 0x10 + 0x10 * i) for i in range(3)]
    csz = struct.unpack_from('<3I', d, 0x60)
    img = bytearray(max(m + s for _, m, s, _ in segs))
    for i, (fo, mo, sz, _) in enumerate(segs):
        if flags & (1 << i):
            import lz4.block
            seg = lz4.block.decompress(bytes(d[fo:fo + csz[i]]), uncompressed_size=sz)
        else:
            seg = bytes(d[fo:fo + sz])
        img[mo:mo + len(seg)] = seg
    return bytes(img)


def scan_main(path, out):
    global OUT
    OUT = out
    low_priority()
    d = open(path, 'rb').read()
    if d[:4] == b'NSO0':
        d = nso_image(d)
    return slim([(os.path.basename(path), 0, 0, 0, 'main', go, si, stage_of(ctrl), ctrl, code, gb)
                 for go, gb, secs in glslc_outputs(d) for si, ctrl, code in secs])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--romfs', required=True)
    ap.add_argument('--romfs-base', help="the base game's romfs, layered under --romfs (files of --romfs win)")
    ap.add_argument('--main')
    ap.add_argument('--out', required=True)
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('--progress', action='store_true', help='print "scan: <done> <total>" lines while scanning')
    a = ap.parse_args()
    by_rel = {}   # path below motor_asset_re -> host path; the update's (--romfs) files win over the base game's
    for top in [a.romfs] + ([a.romfs_base] if a.romfs_base else []):
        root = os.path.join(top, 'motor_asset_re')
        for r, _, fs in os.walk(root):
            for f in fs:
                if f.endswith(('.fdata', '.file')):
                    p = os.path.join(r, f)
                    by_rel.setdefault(os.path.relpath(p, root).replace(os.sep, '/'), p)
    paths = [by_rel[k] for k in sorted(by_rel)]
    for sub in ('code', 'ctrl', 'glslc'):
        os.makedirs(os.path.join(a.out, sub), exist_ok=True)
    seen = {}
    ctrl_variants = collections.defaultdict(set)
    occ = collections.Counter()
    rows = []
    with ProcessPoolExecutor(a.jobs) as ex:
        futs = [ex.submit(scan_file, p, a.out) for p in paths]
        if a.main:
            futs.append(ex.submit(scan_main, a.main, a.out))
        if a.progress:   # "scan: <done> <total>" per finished file (the launcher's setup reads these)
            done = [0]
            def tick(_):
                done[0] += 1
                print(f'scan: {done[0]} {len(futs)}', flush=True)
            for f in futs:
                f.add_done_callback(tick)
        results = [f.result() for f in futs]
    for res in results:
        for name, eo, fk, tk, tag, go, si, st, cn, kn, h, ch, gh in res:
            occ[h] += 1
            ctrl_variants[h].add(ch)
            if h not in seen:
                seen[h] = (st, kn, cn, name, eo, go, ch, gh)
            rows.append(f'{name}\t{eo:#x}\t{fk:08x}\t{tk:08x}\t{tag}\t{go:#x}\t{si}\t{st}\t{cn:#x}\t{kn:#x}\t{h}\t{ch}\t{gh}')
    with open(os.path.join(a.out, 'manifest.tsv'), 'w', newline='\n') as f:
        f.write('source\tentry_off\tfile_ktid\ttype_ktid\tcontainer\tglslc_off\tsection\tstage\tctrl_size\tcode_size'
                '\tcode_sha1_16\tctrl_sha1_16\tglslc_sha1_16\n')
        f.write('\n'.join(rows) + '\n')
    with open(os.path.join(a.out, 'unique.tsv'), 'w', newline='\n') as f:
        f.write('code_sha1_16\tstage\tcode_size\tctrl_size\toccurrences\tctrl_variants\tfirst_source\tentry_off\tglslc_off'
                '\tfirst_ctrl\tfirst_glslc\n')
        for h in sorted(seen):
            st, kn, cn, name, eo, go, ch, gh = seen[h]
            f.write(f'{h}\t{st}\t{kn:#x}\t{cn:#x}\t{occ[h]}\t{len(ctrl_variants[h])}\t{name}\t{eo:#x}\t{go:#x}\t{ch}\t{gh}\n')
    by_all = collections.Counter(r.split('\t')[7] for r in rows)
    by_uni = collections.Counter(v[0] for v in seen.values())
    s = [f'occurrences {len(rows)}  unique code blocks {len(seen)}']
    for st in sorted(set(by_all) | set(by_uni)):
        s.append(f'  {st}: {by_all[st]} occurrences, {by_uni[st]} unique')
    s.append('ctrl sizes (unique code): ' + ', '.join(f'{k:#x}:{v}' for k, v in
                                                     sorted(collections.Counter(v[2] for v in seen.values()).items())))
    open(os.path.join(a.out, 'summary.txt'), 'w', newline='\n').write('\n'.join(s) + '\n')
    print('\n'.join(s))


if __name__ == '__main__':
    main()
