"""Build, inspect, smoke-test and package the desktop releases (stdlib only)."""
from __future__ import annotations

import sys

if sys.version_info < (3, 12):
    raise SystemExit("Release packaging needs Python 3.12 or newer.")

import argparse
import ctypes
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import signal
import struct
import subprocess
import tarfile
import tempfile
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
FFMPEG_VERSION = "9.0.2"
MEDIA = {
    "windows-x64": {
        "ffmpeg-windows.zip": (
            "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip",
            "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba",
        ),
    },
    "linux-x64": {
        "ffmpeg-linux-x64.zip": (
            "https://ffmpeg.martin-riedl.de/download/linux/amd64/1789931100_9.0.2/ffmpeg.zip",
            "fa8ecf4abbd290d98f7d188b8649cc6b391ae209a98452be955a15aab1909d7f",
        ),
        "ffprobe-linux-x64.zip": (
            "https://ffmpeg.martin-riedl.de/download/linux/amd64/1789931100_9.0.2/ffprobe.zip",
            "3f428c49070be3d24ec338602b76d412e401ffcb8a5641ef0e729181a232fc32",
        ),
    },
    "linux-arm64": {
        "ffmpeg-linux-arm64.zip": (
            "https://ffmpeg.martin-riedl.de/download/linux/arm64/1789931697_9.0.2/ffmpeg.zip",
            "93a76ae90db5474eecdf951a729857c64f3de23567228d6a7d5e6e8e3cd1021b",
        ),
        "ffprobe-linux-arm64.zip": (
            "https://ffmpeg.martin-riedl.de/download/linux/arm64/1789931697_9.0.2/ffprobe.zip",
            "bcbe80fb741c180083327afaf5434812e006b33cacde2016b9aeaf6936128330",
        ),
    },
}


class ReleaseError(RuntimeError):
    pass


def contained_path(*, root: Path, path: Path) -> Path:
    root = root.resolve()
    candidate = path if path.is_absolute() else root / path
    if candidate.is_symlink() or candidate.is_junction():
        raise ReleaseError(f"Refusing a linked output path: {candidate}")
    candidate = candidate.resolve()
    if candidate == root or not candidate.is_relative_to(root):
        raise ReleaseError(f"Output must be a subdirectory of {root}: {candidate}")
    return candidate


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def stop_child(child: subprocess.Popen) -> None:
    if child.poll() is not None:
        return
    if os.name == "nt":
        subprocess.run(
            [str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"),
             "/PID", str(child.pid), "/T", "/F"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
        )
    else:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    child.wait()


def run(args: list[str], *, env: dict[str, str] | None = None,
        capture: bool = False, timeout: int | None = None) -> str:
    child = subprocess.Popen(
        args, cwd=ROOT, env=env, text=True, encoding="utf-8", errors="replace",
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        start_new_session=os.name != "nt",
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0,
    )
    try:
        stdout, stderr = child.communicate(timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt):
        stop_child(child)
        raise ReleaseError(f"Command cancelled or timed out: {args[0]}") from None
    if child.returncode:
        raise ReleaseError(f"Command failed ({child.returncode}): {args!r}\n{stderr or ''}{stdout or ''}")
    return stdout or ""


def build_env(*, target: str) -> dict[str, str]:
    env = os.environ.copy()
    for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTUP_TOOLCHAIN"):
        env.pop(name, None)
    cpu = "x86-64" if target.startswith("x86_64") else "generic"
    flags = ["-C", f"target-cpu={cpu}"]
    if "windows" in target:
        flags += ["-C", "target-feature=+crt-static"]
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
    return env


def cargo(*, channel: str, args: list[str], env: dict[str, str], capture: bool = False) -> str:
    return run(["rustup", "run", channel, "cargo", *args], env=env, capture=capture)


def preflight(*, system: str, destination: Path) -> dict:
    if (system == "windows" and sys.platform != "win32") or (system == "linux" and sys.platform != "linux"):
        raise ReleaseError(f"The {system} build must run on {system}.")
    if sys.platform == "win32":
        # platform.machine() uses WMI on recent Python and can be empty under a sandbox.
        native_info = ctypes.create_string_buffer(64)
        ctypes.windll.kernel32.GetNativeSystemInfo(ctypes.byref(native_info))
        arch = {9: "x64", 12: "arm64"}.get(struct.unpack_from("<H", native_info.raw)[0])
    else:
        arch = {"x86_64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(platform.machine().lower())
    if arch is None or (system == "windows" and arch != "x64"):
        raise ReleaseError(f"Unsupported {system} build host: {platform.machine()}")
    target = ("x86_64-pc-windows-msvc" if system == "windows" else
              ("x86_64-unknown-linux-gnu" if arch == "x64" else "aarch64-unknown-linux-gnu"))
    tools = ["rustup"] if system == "windows" else ["rustup", "cc", "c++", "pkg-config", "readelf", "ldd", "getconf"]
    for tool in tools:
        if not shutil.which(tool):
            raise ReleaseError(f"Missing {tool}. See README.md release build prerequisites.")
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"]
    rustc = run(["rustup", "run", channel, "rustc", "--version"], capture=True).strip()
    installed = run(["rustup", "target", "list", "--installed", "--toolchain", channel], capture=True).splitlines()
    if target not in installed:
        raise ReleaseError(f"Install the pinned target: rustup target add {target} --toolchain {channel}")
    baseline = None
    if system == "windows":
        vswhere = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"
        if not vswhere.is_file() or not run(
            [str(vswhere), "-latest", "-products", "*", "-requires",
             "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], capture=True,
        ).strip():
            raise ReleaseError("Install Visual Studio C++ Build Tools with the x64 tools and Windows SDK.")
    else:
        run(["pkg-config", "--exists", "xkbcommon", "wayland-client", "xcb-render", "xcb-shape", "xcb-xfixes"], capture=True)
        baseline = run(["getconf", "GNU_LIBC_VERSION"], capture=True).strip().split()[-1]
        if arch == "x64" and baseline != "2.39":
            raise ReleaseError("The x64 Linux release needs the Ubuntu 24.04/glibc 2.39 build environment.")
    for required in ("docs/package-readme.md", "docs/screenshots/split-view.png", "LICENSES/ffmpeg-GPL-3.0.txt"):
        if not (ROOT / required).is_file():
            raise ReleaseError(f"Missing package input: {required}")
    env = build_env(target=target)
    metadata = json.loads(cargo(channel=channel, args=["metadata", "--format-version", "1", "--no-deps", "--locked"], env=env, capture=True))
    versions = {p["name"]: p["version"] for p in metadata["packages"] if p["name"] in {"ascii-painter-gui", "ascii-painter-cli"}}
    if len(versions) != 2 or len(set(versions.values())) != 1:
        raise ReleaseError(f"GUI and CLI versions must agree: {versions}")
    version = versions["ascii-painter-gui"]
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        raise ReleaseError(f"Unsafe package version: {version}")
    source = {"revision": None, "dirty": None}
    if shutil.which("git"):
        try:
            prefix = ["git", "-c", f"safe.directory={ROOT}"]
            source = {"revision": run([*prefix, "rev-parse", "HEAD"], capture=True).strip(),
                      "dirty": bool(run([*prefix, "status", "--porcelain"], capture=True).strip())}
        except ReleaseError:
            pass
    contained_path(root=ROOT, path=destination)
    return {"version": version, "platform": system, "architecture": arch, "target": target,
            "channel": channel, "rustc": rustc, "glibc": baseline, "source": source}


def package_name(info: dict) -> str:
    name = f"ascii-painter-{info['version']}-{info['platform']}-{info['architecture']}"
    return name + (f"-glibc{info['glibc']}" if info["platform"] == "linux" else "")


def fetch(*, path: Path, url: str, expected: str) -> Path:
    path = contained_path(root=ROOT, path=path)
    if path.is_file() and digest(path) == expected:
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=path.name + ".", suffix=".part", dir=path.parent)
    temporary = Path(temporary)
    try:
        with os.fdopen(descriptor, "wb") as output, urllib.request.urlopen(url, timeout=30) as response:
            shutil.copyfileobj(response, output)
        actual = digest(temporary)
        if actual != expected:
            raise ReleaseError(f"FFmpeg checksum mismatch: expected {expected}, got {actual}")
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)
    return path


def install_media(*, folder: Path, info: dict) -> None:
    archives = {name: fetch(path=ROOT / "dist/cache" / name, url=url, expected=sha)
                for name, (url, sha) in MEDIA[f"{info['platform']}-{info['architecture']}"].items()}
    for tool in ("ffmpeg", "ffprobe"):
        windows = info["platform"] == "windows"
        archive = archives["ffmpeg-windows.zip" if windows else f"{tool}-linux-{info['architecture']}.zip"]
        member = f"ffmpeg-9.0.2-essentials_build/bin/{tool}.exe" if windows else tool
        destination = folder / (tool + (".exe" if windows else ""))
        with zipfile.ZipFile(archive) as z, z.open(member) as source, destination.open("wb") as output:
            shutil.copyfileobj(source, output)
        destination.chmod(0o755)


def pe_imports(path: Path) -> list[str]:
    data = path.read_bytes()
    if data[:2] != b"MZ":
        raise ReleaseError(f"Not a PE executable: {path}")
    pe = struct.unpack_from("<I", data, 0x3c)[0]
    if data[pe:pe+4] != b"PE\0\0" or struct.unpack_from("<H", data, pe+4)[0] != 0x8664:
        raise ReleaseError(f"Not a Windows x64 executable: {path}")
    count = struct.unpack_from("<H", data, pe+6)[0]
    optional_size = struct.unpack_from("<H", data, pe+20)[0]
    optional = pe+24
    if struct.unpack_from("<H", data, optional)[0] != 0x20b:
        raise ReleaseError(f"Not PE32+: {path}")
    sections = [struct.unpack_from("<IIII", data, optional+optional_size+i*40+8) for i in range(count)]

    def offset(rva: int) -> int:
        for virtual_size, address, raw_size, raw_offset in sections:
            if address <= rva < address + max(virtual_size, raw_size):
                return raw_offset+rva-address
        if rva < optional+optional_size:
            return rva
        raise ReleaseError(f"Invalid PE address in {path}")

    names = set()
    for directory, size in ((1, 20), (13, 32)):
        rva = struct.unpack_from("<I", data, optional+112+directory*8)[0]
        if not rva:
            continue
        cursor = offset(rva)
        while any(data[cursor:cursor+size]):
            name_rva = struct.unpack_from("<I", data, cursor+(12 if directory == 1 else 4))[0]
            if directory == 13 and not struct.unpack_from("<I", data, cursor)[0] & 1:
                name_rva -= struct.unpack_from("<Q", data, optional+24)[0]
            start = offset(name_rva)
            names.add(data[start:data.index(b"\0", start)].decode("ascii").lower())
            cursor += size
    return sorted(names)


def version_tuple(value: str) -> tuple[int, ...]:
    return tuple(int(part) for part in value.split("."))


def audit(*, folder: Path, info: dict) -> dict:
    result = {}
    linux_libraries = {"libgcc_s.so.1", "libm.so.6", "libc.so.6", "libmvec.so.1", "libstdc++.so.6",
                       "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1", "libdl.so.2", "libpthread.so.0", "librt.so.1"}
    for tool in ("ascii-painter", "ascii-painter-cli", "ffmpeg", "ffprobe"):
        path = folder / (tool + (".exe" if info["platform"] == "windows" else ""))
        if info["platform"] == "windows":
            imports = pe_imports(path)
            for library in imports:
                if re.match(r"(?:vcruntime|msvcp|msvcr|concrt)\d", library):
                    raise ReleaseError(f"{tool} still needs the Visual C++ redistributable: {library}")
                if not library.startswith(("api-ms-win-", "ext-ms-win-")) and not (Path(os.environ["SystemRoot"]) / "System32" / library).is_file():
                    raise ReleaseError(f"Unbundled Windows dependency in {tool}: {library}")
            result[tool] = {"imports": imports}
        else:
            header = run(["readelf", "--file-header", str(path)], capture=True)
            machine = "Advanced Micro Devices X86-64" if info["architecture"] == "x64" else "AArch64"
            if not re.search(r"Class:\s+ELF64", header) or not re.search(r"Machine:\s+" + re.escape(machine), header):
                raise ReleaseError(f"Wrong ELF architecture: {path}")
            dynamic = run(["readelf", "--dynamic", str(path)], capture=True)
            libraries = re.findall(r"Shared library: \[([^]]+)\]", dynamic)
            if set(libraries) - linux_libraries or re.search(r"\((?:RPATH|RUNPATH)\)", dynamic):
                raise ReleaseError(f"Unexpected Linux dependency or runtime search path in {tool}: {libraries}")
            versions = run(["readelf", "--version-info", str(path)], capture=True)
            requirements = sorted(set(re.findall(r"Name: ((?:GLIBC|GLIBCXX|CXXABI|GCC)_[0-9.]+)", versions)))
            limits = {"GLIBC": info["glibc"]}
            if info["architecture"] == "x64":
                limits.update({"GLIBCXX": "3.4.30", "CXXABI": "1.3.13"})
            for requirement in requirements:
                family, required = requirement.split("_", 1)
                if family in limits and version_tuple(required) > version_tuple(limits[family]):
                    raise ReleaseError(f"{tool} requires {requirement}, above the release baseline {family}_{limits[family]}")
            if libraries and "not found" in run(["ldd", str(path)], capture=True):
                raise ReleaseError(f"Unresolved Linux runtime library in {tool}")
            result[tool] = {"libraries": libraries, "symbol_versions": requirements}
    return result


def write_archive(*, folder: Path, archive: Path, windows: bool) -> None:
    files = [folder, *sorted(folder.rglob("*"))]
    executables = {"ascii-painter", "ascii-painter-cli", "ffmpeg", "ffprobe"}
    if any(path.is_symlink() or path.is_junction() for path in files):
        raise ReleaseError("Packages may contain only directories and regular files.")
    if windows:
        with zipfile.ZipFile(archive, mode="w", compression=zipfile.ZIP_DEFLATED) as z:
            for path in files:
                z.write(filename=path, arcname=path.relative_to(folder.parent).as_posix())
    else:
        with tarfile.open(name=archive, mode="w:gz", format=tarfile.PAX_FORMAT) as tar:
            for path in files:
                entry = tar.gettarinfo(name=path, arcname=path.relative_to(folder.parent).as_posix())
                entry.uid = entry.gid = 0
                entry.uname = entry.gname = ""
                entry.mode = 0o755 if path.is_dir() or path.relative_to(folder).as_posix() in executables else 0o644
                if path.is_dir():
                    tar.addfile(tarinfo=entry)
                else:
                    with path.open("rb") as stream:
                        tar.addfile(tarinfo=entry, fileobj=stream)


def validate_member_names(*, names: list[str], name: str) -> None:
    if len(names) != len(set(names)):
        raise ReleaseError("Duplicate archive entries.")
    for member in names:
        path = PurePosixPath(member)
        if path.is_absolute() or not path.parts or path.parts[0] != name or ".." in path.parts or "\\" in member:
            raise ReleaseError(f"Unexpected archive path: {member}")


def extract_checked(*, archive: Path, destination: Path, name: str, windows: bool) -> Path:
    if windows:
        with zipfile.ZipFile(archive) as z:
            if z.testzip() is not None:
                raise ReleaseError(f"Corrupt ZIP: {archive}")
            validate_member_names(names=[i.filename.rstrip("/") for i in z.infolist()], name=name)
            z.extractall(path=destination)
    else:
        with tarfile.open(name=archive, mode="r:gz") as tar:
            entries = tar.getmembers()
            validate_member_names(names=[i.name for i in entries], name=name)
            executables = {f"{name}/{n}" for n in ("ascii-painter", "ascii-painter-cli", "ffmpeg", "ffprobe")}
            for entry in entries:
                expected = 0o755 if entry.isdir() or entry.name in executables else 0o644
                if not (entry.isdir() or entry.isfile()) or entry.mode != expected or entry.uid != 0 or entry.gid != 0:
                    raise ReleaseError(f"Incorrect tar entry, permissions or ownership: {entry.name}")
            tar.extractall(path=destination, filter="data")
    return destination / name


def smoke(*, folder: Path, work: Path, info: dict) -> None:
    work.mkdir()
    env = os.environ.copy()
    env["PATH"] = ""  # Media discovery must use the adjacent packaged tools.
    env.pop("LD_LIBRARY_PATH", None)
    env.pop("LD_PRELOAD", None)
    env.update({"XDG_CONFIG_HOME": str(work / "config"), "XDG_DATA_HOME": str(work / "data"),
                "APPDATA": str(work / "config"), "LOCALAPPDATA": str(work / "data")})
    suffix = ".exe" if info["platform"] == "windows" else ""

    def execute(tool: str, args: list[str], *, timeout: int = 60) -> str:
        return run([str(folder / (tool+suffix)), *args], env=env, capture=True, timeout=timeout)

    if execute("ascii-painter-cli", ["--version"]).strip().split()[-1] != info["version"]:
        raise ReleaseError("The packaged CLI version does not match the filename.")
    for tool in ("ffmpeg", "ffprobe"):
        if not execute(tool, ["-version"]).startswith(f"{tool} version {FFMPEG_VERSION}"):
            raise ReleaseError(f"Wrong packaged {tool} version.")
    source, image, still, video = [work / name for name in ("input.mp4", "input.png", "still.png", "video.mp4")]
    execute("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=64x48:rate=3:duration=1",
                       "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100:duration=1", "-shortest",
                       "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", str(source)])
    execute("ffmpeg", ["-v", "error", "-i", str(source), "-frames:v", "1", str(image)])
    execute("ascii-painter-cli", ["convert", str(image), "-o", str(still), "--cols", "8"])
    execute("ascii-painter-cli", ["convert", str(source), "-o", str(video), "--cols", "8"])
    streams = json.loads(execute("ffprobe", ["-v", "error", "-show_streams", "-of", "json", str(video)]))["streams"]
    encoded = next((stream for stream in streams if stream["codec_type"] == "video"), {})
    if encoded.get("codec_name") != "h264" or encoded.get("nb_frames") != "3" or not any(s.get("codec_name") == "aac" for s in streams):
        raise ReleaseError("Packaged video export did not retain three H.264 frames and AAC audio.")
    snapshot = work / "gui.png"
    execute("ascii-painter", ["--scene", "image", "--snap", str(snapshot), "--size", "900x600", "--at", "1.0"], timeout=90)
    for path in (still, snapshot):
        if not path.is_file() or not path.read_bytes().startswith(b"\x89PNG\r\n\x1a\n"):
            raise ReleaseError(f"Missing packaged PNG output: {path}")


def build_native(*, destination: Path, info: dict) -> Path:
    windows = info["platform"] == "windows"
    name = package_name(info)
    output = contained_path(root=ROOT, path=destination / "native" / f"{info['platform']}-{info['architecture']}")
    output.mkdir(parents=True, exist_ok=True)
    target_root = Path(os.environ.get("CARGO_TARGET_DIR", "target/package")) / f"{info['platform']}-{info['architecture']}"
    target_root = contained_path(root=ROOT, path=target_root)
    print(f"==> Building {name}", flush=True)
    cargo(channel=info["channel"], args=["build", "--release", "--workspace", "--locked", "--target", info["target"], "--target-dir", str(target_root)], env=build_env(target=info["target"]))
    with tempfile.TemporaryDirectory(prefix=".package-", dir=output) as temporary:
        stage = Path(temporary)
        folder = stage / name
        folder.mkdir()
        for tool in ("ascii-painter", "ascii-painter-cli"):
            binary = tool + (".exe" if windows else "")
            shutil.copyfile(src=target_root / info["target"] / "release" / binary, dst=folder / binary)
            (folder / binary).chmod(0o755)
        print("==> Checking pinned media tools", flush=True)
        install_media(folder=folder, info=info)
        shutil.copyfile(src=ROOT / "docs/package-readme.md", dst=folder / "README.md")
        shutil.copytree(src=ROOT / "LICENSES", dst=folder / "LICENSES")
        (folder / "docs/screenshots").mkdir(parents=True)
        shutil.copyfile(src=ROOT / "docs/screenshots/split-view.png", dst=folder / "docs/screenshots/split-view.png")
        dependencies = audit(folder=folder, info=info)
        build_info = {**info, "built_at": datetime.now(timezone.utc).isoformat(), "ffmpeg_version": FFMPEG_VERSION,
                      "static_windows_crt": windows, "dependencies": dependencies,
                      "media_sha256": {name: sha for name, (_, sha) in MEDIA[f"{info['platform']}-{info['architecture']}"].items()}}
        (folder / "BUILD-INFO.json").write_text(json.dumps(build_info, indent=2)+"\n", encoding="utf-8", newline="\n")
        archive = stage / (name + (".zip" if windows else ".tar.gz"))
        write_archive(folder=folder, archive=archive, windows=windows)
        extracted = extract_checked(archive=archive, destination=stage / "extracted", name=name, windows=windows)
        if {p.relative_to(folder) for p in folder.rglob("*")} != {p.relative_to(extracted) for p in extracted.rglob("*")}:
            raise ReleaseError("Archive contents differ from the assembled package.")
        print("==> Smoke-testing extracted image/video conversion and GUI rendering", flush=True)
        audit(folder=extracted, info=info)
        smoke(folder=extracted, work=stage / "smoke", info=info)
        final = contained_path(root=ROOT, path=output / archive.name)
        os.replace(archive, final)
    print(f"==> Verified package: {final}", flush=True)
    return final


def write_checksums(*, directory: Path, archives: list[Path], version: str) -> Path:
    manifest = directory / f"ascii-painter-{version}-SHA256SUMS.txt"
    manifest.write_text("".join(f"{digest(path)}  {path.name}\n" for path in sorted(archives)), encoding="utf-8", newline="\n")
    return manifest


def verify_release_directory(*, directory: Path, version: str) -> None:
    manifest = directory / f"ascii-painter-{version}-SHA256SUMS.txt"
    names = {f"ascii-painter-{version}-windows-x64.zip", f"ascii-painter-{version}-linux-x64-glibc2.39.tar.gz"}
    if directory.is_symlink() or directory.is_junction() or {p.name for p in directory.iterdir()} != names | {manifest.name}:
        raise ReleaseError(f"Not an owned complete release directory: {directory}")
    if any(not p.is_file() or p.is_symlink() or p.is_junction() for p in directory.iterdir()):
        raise ReleaseError(f"Unexpected release entries: {directory}")
    expected = "".join(f"{digest(directory/name)}  {name}\n" for name in sorted(names)).encode("utf-8")
    if manifest.read_bytes() != expected:
        raise ReleaseError(f"Release checksums do not match: {directory}")


def publish_release(*, staged: Path, destination: Path, version: str) -> Path:
    releases = contained_path(root=ROOT, path=destination / "releases")
    releases.mkdir(parents=True, exist_ok=True)
    final = contained_path(root=ROOT, path=releases / version)
    backup = contained_path(root=ROOT, path=releases / f".previous-{version}")
    if backup.exists():
        raise ReleaseError(f"An earlier release backup needs recovery before rebuilding: {backup}")
    verify_release_directory(directory=staged, version=version)
    had_previous = final.exists()
    if had_previous:
        verify_release_directory(directory=final, version=version)
        final.rename(target=backup)
    try:
        staged.rename(target=final)
    except BaseException:
        if had_previous:
            try:
                backup.rename(target=final)
            except OSError:
                print(f"Previous release retained for recovery: {backup}", file=sys.stderr)
        raise
    if had_previous:
        verify_release_directory(directory=backup, version=version)
        try:
            shutil.rmtree(path=contained_path(root=ROOT, path=backup))
        except OSError:
            print(f"Release published; previous backup retained at {backup}", file=sys.stderr)
    return final


@contextmanager
def release_lock(destination: Path):
    destination.mkdir(parents=True, exist_ok=True)
    lock_path = contained_path(root=ROOT, path=destination / ".release.lock")
    with lock_path.open("a+b") as stream:
        stream.seek(0, os.SEEK_END)
        if stream.tell() == 0:
            stream.write(b"0")
            stream.flush()
        stream.seek(0)
        if os.name == "nt":
            import msvcrt
            try:
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            except OSError:
                raise ReleaseError(f"Another release is using {destination}") from None
        else:
            import fcntl
            try:
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                raise ReleaseError(f"Another release is using {destination}") from None
        yield


def build_all(*, destination: Path, distro: str, user: str | None) -> None:
    if sys.platform != "win32" or not shutil.which("wsl.exe"):
        raise ReleaseError("The combined release command needs Windows x64 and an installed WSL distribution.")
    prefix = ["wsl.exe", "--distribution", distro]
    if user:
        prefix += ["--user", user]
    prefix += ["--cd", str(ROOT), "--exec"]
    with release_lock(destination):
        windows = preflight(system="windows", destination=destination)
        linux = json.loads(run([*prefix, "bash", "./build.sh", "--preflight"], capture=True).strip())
        if linux["architecture"] != "x64" or linux["glibc"] != "2.39" or linux["version"] != windows["version"]:
            raise ReleaseError("WSL must provide x64/glibc 2.39 and the same application version.")
        version = windows["version"]
        backup = contained_path(root=ROOT, path=destination / "releases" / f".previous-{version}")
        if backup.exists():
            raise ReleaseError(f"Recover the previous release backup before rebuilding: {backup}")
        existing = contained_path(root=ROOT, path=destination / "releases" / version)
        if existing.exists():
            verify_release_directory(directory=existing, version=version)
        staging = contained_path(root=ROOT, path=destination / ".staging")
        staging.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="release-", dir=staging) as temporary:
            stage = Path(temporary)
            native = stage / "build"
            win_archive = build_native(destination=native, info=windows)
            linux_destination = run([*prefix, "wslpath", "-a", "-u", str(native)], capture=True).strip()
            run([*prefix, "env", f"ASCII_PAINTER_DIST_DIR={linux_destination}", "bash", "./build.sh"])
            linux_archive = native / "native/linux-x64" / (package_name(linux)+".tar.gz")
            complete = stage / "complete"
            complete.mkdir()
            archives = []
            for archive in (win_archive, linux_archive):
                target = complete / archive.name
                shutil.copyfile(src=archive, dst=target)
                archives.append(target)
            write_checksums(directory=complete, archives=archives, version=version)
            final = publish_release(staged=complete, destination=destination, version=version)
        print("==> Release verified. Upload these files to GitHub:", flush=True)
        for path in sorted(final.iterdir()):
            print(path, flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["build"])
    parser.add_argument("--platform", required=True, choices=["windows", "linux", "all"])
    parser.add_argument("--destination-root", default="dist")
    parser.add_argument("--wsl-distro", default="Ubuntu")
    parser.add_argument("--wsl-user")
    parser.add_argument("--preflight", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    try:
        destination = contained_path(root=ROOT, path=Path(args.destination_root))
        if args.platform == "all":
            build_all(destination=destination, distro=args.wsl_distro, user=args.wsl_user)
        else:
            info = preflight(system=args.platform, destination=destination)
            if args.preflight:
                print(json.dumps(info))
            else:
                build_native(destination=destination, info=info)
    except (ReleaseError, OSError, ValueError, KeyError, zipfile.BadZipFile, tarfile.TarError) as error:
        print(f"ERROR: {error}", file=sys.stderr, flush=True)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
