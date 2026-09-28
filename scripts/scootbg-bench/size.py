"""The Size and Disk rows.

Size is the ticket's: the stripped executables plus their shared-library
closure as ``ldd`` resolves it, glibc's own libraries left out (``libc``,
``libm``, the loader and the rest of the glibc package: every daemon has
them). Each file is stripped (``strip --strip-all``) into a scratch copy
before it is weighed, so a package that ships unstripped binaries is not
charged for its symbols. Libraries a daemon ``dlopen``s (gdk-pixbuf's
loaders, Mesa's drivers) are invisible to ``ldd``; the idle runs record
what each daemon actually mapped, and ``runtime_mapped`` weighs that too,
reported beside the row, not in it.

Disk is the installed package (``du`` of its store path: binaries, man
pages, completions) plus what the daemon writes under its home, cache and
state directories, measured after a live change in ``scenarios.live_set``.
"""

import os
import shutil
import subprocess
import tempfile


def _stripped_size(path, strip):
    with tempfile.TemporaryDirectory(prefix="sbbsz") as d:
        copy = os.path.join(d, "f")
        shutil.copyfile(os.path.realpath(path), copy)
        r = subprocess.run([strip, "--strip-all", copy], capture_output=True)
        if r.returncode != 0:
            return os.path.getsize(os.path.realpath(path))
        return os.path.getsize(copy)


def interpreter(binary):
    """The ELF's ``PT_INTERP`` (its dynamic loader), or ``None``."""
    import struct

    with open(binary, "rb") as f:
        ident = f.read(16)
        if ident[:4] != b"\x7fELF" or ident[4] != 2:  # 64-bit only
            return None
        end = "<" if ident[5] == 1 else ">"
        f.seek(0x20)
        (phoff,) = struct.unpack(end + "Q", f.read(8))
        f.seek(0x36)
        phentsize, phnum = struct.unpack(end + "HH", f.read(4))
        for i in range(phnum):
            f.seek(phoff + i * phentsize)
            p_type, _flags, p_offset = struct.unpack(end + "IIQ", f.read(16))
            if p_type == 3:  # PT_INTERP
                f.seek(p_offset + 0)
                f.seek(phoff + i * phentsize + 32)
                (p_filesz,) = struct.unpack(end + "Q", f.read(8))
                f.seek(p_offset)
                return f.read(p_filesz).rstrip(b"\0").decode()
    return None


def ldd_closure(binary):
    """Resolved library paths, as ``ldd`` gives them, from the binary's own
    loader (``ld.so --list``): the host's ``ldd`` would resolve a nix
    binary against the host's libraries. ``LD_LIBRARY_PATH`` is cleared so
    the dev shell's does not change the answer."""
    env = {k: v for k, v in os.environ.items() if k != "LD_LIBRARY_PATH"}
    interp = interpreter(os.path.realpath(binary))
    argv = [interp, "--list", binary] if interp else ["ldd", binary]
    r = subprocess.run(argv, capture_output=True, text=True, env=env)
    libs = []
    for line in r.stdout.splitlines():
        parts = line.split("=>")
        target = parts[1].strip().split(" (")[0] if len(parts) == 2 else parts[0].strip().split(" (")[0]
        if target.startswith("/"):
            libs.append(os.path.realpath(target))
    return libs


def _glibc_dirs(libs):
    return {os.path.dirname(p) for p in libs if os.path.basename(p).startswith(("libc.so", "ld-linux"))}


def _is_glibc(path, glibc_dirs):
    return os.path.dirname(os.path.realpath(path)) in glibc_dirs


def weigh(binaries, strip, extra_libraries=()):
    """Size of the stripped binaries and their non-glibc ``ldd`` closure,
    with every file listed; ``extra_libraries`` (mapped at run time) are
    weighed separately."""
    closure = set()
    for b in binaries:
        closure.update(ldd_closure(b))
    glibc = _glibc_dirs(closure)
    files = []
    total = 0
    for b in binaries:
        size = _stripped_size(b, strip)
        files.append([os.path.realpath(b), size, "binary"])
        total += size
    libs_total = 0
    for lib in sorted(closure):
        if _is_glibc(lib, glibc):
            continue
        size = _stripped_size(lib, strip)
        files.append([lib, size, "ldd"])
        libs_total += size
    runtime_total = 0
    for lib in sorted({os.path.realpath(p) for p in extra_libraries} - closure):
        if _is_glibc(lib, glibc) or not os.path.exists(lib):
            continue
        size = _stripped_size(lib, strip)
        files.append([lib, size, "runtime-only"])
        runtime_total += size
    return {
        "binaries_bytes": total,
        "ldd_libraries_bytes": libs_total,
        "size_bytes": total + libs_total,
        "runtime_only_bytes": runtime_total,
        "files": files,
    }


def installed_bytes(store_path):
    """``du`` of a package's store path (apparent sizes)."""
    total = 0
    for dirpath, _dirs, names in os.walk(store_path):
        for n in names:
            p = os.path.join(dirpath, n)
            if not os.path.islink(p):
                total += os.path.getsize(p)
    return total


def _path_info(path):
    """``{store path: nar size}`` for the runtime closure of ``path``."""
    argv = ["nix", "path-info", "-r", "--json", path]
    r = subprocess.run(argv[:4] + ["--json-format", "1", path], capture_output=True, text=True)
    if r.returncode != 0:  # an older nix without --json-format
        r = subprocess.run(argv, capture_output=True, text=True, check=True)
    data = __import__("json").loads(r.stdout)
    if isinstance(data, dict):
        return {p: v["narSize"] for p, v in data.items()}
    return {v["path"]: v["narSize"] for v in data}


def closure_bytes(store_path, binaries):
    """The package's runtime closure, less glibc's own closure (every
    daemon needs glibc; the gcc runtime, cairo, Mesa's loader and the rest
    are counted), with the paths kept."""
    closure = _path_info(store_path)
    libs = set()
    for b in binaries:
        libs.update(ldd_closure(b))
    glibc_roots = {"/".join(p.split("/")[:4]) for p in _glibc_dirs(libs)}
    glibc = {}
    for root in glibc_roots:
        glibc.update(_path_info(root))
    kept = {p: n for p, n in closure.items() if p not in glibc}
    return {"closure_bytes": sum(kept.values()), "closure_paths": sorted(kept.items())}
