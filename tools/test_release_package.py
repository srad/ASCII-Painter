"""Focused package boundary and failure tests; no compiler, downloads or GPU needed."""
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

import release_package as release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        cache = release.ROOT / "target/tmp"
        cache.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="packaging-tests-", dir=cache)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        patch = mock.patch.object(release, "ROOT", self.root)
        patch.start()
        self.addCleanup(patch.stop)

    def fixture_release(self, *, directory, value=b"new"):
        directory.mkdir(parents=True)
        archives = []
        for name in ("ascii-painter-0.1.0-windows-x64.zip", "ascii-painter-0.1.0-linux-x64-glibc2.39.tar.gz"):
            path = directory / name
            path.write_bytes(value)
            archives.append(path)
        release.write_checksums(directory=directory, archives=archives, version="0.1.0")
        return directory

    def test_filename_uses_full_version_and_platform_baseline(self):
        info = {"version": "1.2.3-rc.1+build.5", "platform": "linux", "architecture": "x64", "glibc": "2.39"}
        self.assertEqual(release.package_name(info), "ascii-painter-1.2.3-rc.1+build.5-linux-x64-glibc2.39")

    def test_output_rejects_workspace_root_and_parent(self):
        for path in (self.root, self.root / "../escape"):
            with self.assertRaises(release.ReleaseError):
                release.contained_path(root=self.root, path=path)
        self.assertEqual(release.contained_path(root=self.root, path=Path("space and apostrophe's/output")), self.root / "space and apostrophe's/output")

    def test_linked_parent_cannot_escape_root(self):
        workspace = self.root / "workspace"
        outside = self.root / "outside"
        workspace.mkdir()
        outside.mkdir()
        link = workspace / "link"
        if os.name == "nt":
            subprocess.run(["cmd.exe", "/c", "mklink", "/J", str(link), str(outside)], check=True, capture_output=True)
        else:
            link.symlink_to(target=outside, target_is_directory=True)
        with self.assertRaises(release.ReleaseError):
            release.contained_path(root=workspace, path=link / "child")

    def test_archive_roundtrip_and_linux_modes_ignore_source_permissions(self):
        folder = self.root / "package"
        (folder / "docs").mkdir(parents=True)
        for name in ("ascii-painter", "ascii-painter-cli", "ffmpeg", "ffprobe", "README.md", "docs/image.png"):
            path = folder / name
            path.write_bytes(name.encode())
            path.chmod(mode=0o777)
        for windows in (False, True):
            archive = self.root / ("out.zip" if windows else "out.tar.gz")
            release.write_archive(folder=folder, archive=archive, windows=windows)
            extracted = release.extract_checked(archive=archive, destination=self.root / f"unpacked-{windows}", name="package", windows=windows)
            self.assertEqual((extracted / "README.md").read_bytes(), b"README.md")
        with tarfile.open(name=self.root / "out.tar.gz") as archive:
            for entry in archive.getmembers():
                expected = 0o755 if entry.isdir() or entry.name.split("/")[-1] in {"ascii-painter", "ascii-painter-cli", "ffmpeg", "ffprobe"} else 0o644
                self.assertEqual((entry.mode, entry.uid, entry.gid), (expected, 0, 0))

    def test_unsafe_duplicate_and_world_writable_archive_entries_rejected(self):
        for names in (["package/../escape"], ["/package/file"], ["other/file"], ["package/file", "package/file"]):
            with self.assertRaises(release.ReleaseError):
                release.validate_member_names(names=names, name="package")
        archive = self.root / "bad.tar.gz"
        with tarfile.open(name=archive, mode="w:gz") as output:
            entry = tarfile.TarInfo(name="package/README.md")
            entry.mode = 0o777
            output.addfile(tarinfo=entry)
        with self.assertRaises(release.ReleaseError):
            release.extract_checked(archive=archive, destination=self.root / "unpacked", name="package", windows=False)

    def test_bad_download_checksum_preserves_existing_cache(self):
        cached = self.root / "cache/tool.zip"
        cached.parent.mkdir()
        cached.write_bytes(b"existing")
        with mock.patch.object(release.urllib.request, "urlopen", return_value=io.BytesIO(b"bad download")):
            with self.assertRaisesRegex(release.ReleaseError, "checksum mismatch"):
                release.fetch(path=cached, url="https://example.invalid/tool.zip", expected="0" * 64)
        self.assertEqual(cached.read_bytes(), b"existing")
        self.assertEqual(list(cached.parent.iterdir()), [cached])

    def test_missing_tool_and_failed_child_fail_explicitly(self):
        with mock.patch.object(release.shutil, "which", return_value=None):
            with self.assertRaisesRegex(release.ReleaseError, "Missing rustup"):
                release.preflight(system="windows" if sys.platform == "win32" else "linux", destination=self.root / "dist")
        with self.assertRaisesRegex(release.ReleaseError, "Command failed \\(7\\)"):
            release.run([sys.executable, "-c", "raise SystemExit(7)"], capture=True)

    def test_crt_flags_are_scoped_to_windows_and_do_not_change_environment(self):
        before = os.environ.copy()
        windows = release.build_env(target="x86_64-pc-windows-msvc")
        linux = release.build_env(target="x86_64-unknown-linux-gnu")
        self.assertIn("+crt-static", windows["CARGO_ENCODED_RUSTFLAGS"])
        self.assertNotIn("crt-static", linux["CARGO_ENCODED_RUSTFLAGS"])
        self.assertEqual(dict(os.environ), before)

    def test_linux_version_check_is_numeric_and_audits_media_tools(self):
        info = {"platform": "linux", "architecture": "x64", "glibc": "2.39"}
        def readelf(args, **kwargs):
            if "--file-header" in args:
                return "Class: ELF64\nMachine: Advanced Micro Devices X86-64"
            if "--dynamic" in args:
                return ""
            return "Name: GLIBC_2.40" if Path(args[-1]).name == "ffprobe" else "Name: GLIBC_2.9"
        with mock.patch.object(release, "run", side_effect=readelf):
            with self.assertRaisesRegex(release.ReleaseError, "ffprobe requires GLIBC_2.40"):
                release.audit(folder=self.root, info=info)

    def test_release_replacement_checksums_and_other_versions(self):
        destination = self.root / "dist"
        old = self.fixture_release(directory=destination / "releases/0.1.0", value=b"old")
        other = destination / "releases/0.0.1"
        other.mkdir()
        (other / "keep").write_bytes(b"history")
        staged = self.fixture_release(directory=self.root / "staged")
        result = release.publish_release(staged=staged, destination=destination, version="0.1.0")
        self.assertEqual(result, old)
        release.verify_release_directory(directory=result, version="0.1.0")
        self.assertEqual((other / "keep").read_bytes(), b"history")
        self.assertFalse((destination / "releases/.previous-0.1.0").exists())
        manifest = (result / "ascii-painter-0.1.0-SHA256SUMS.txt").read_bytes()
        self.assertFalse(manifest.startswith(b"\xef\xbb\xbf"))
        self.assertNotIn(b"\r", manifest)
        (result / "ascii-painter-0.1.0-windows-x64.zip").write_bytes(b"changed")
        with self.assertRaisesRegex(release.ReleaseError, "checksums do not match"):
            release.verify_release_directory(directory=result, version="0.1.0")

    def test_arm64_retains_its_native_runtime_without_x64_cpp_limits(self):
        def readelf(args, **kwargs):
            if "--file-header" in args:
                return "Class: ELF64\nMachine: AArch64"
            if "--dynamic" in args:
                return "Shared library: [libstdc++.so.6]"
            if args[0] == "ldd":
                return "libstdc++.so.6 => /lib/libstdc++.so.6"
            return "Name: GLIBC_2.35\nName: GLIBCXX_3.4.32"
        with mock.patch.object(release, "run", side_effect=readelf):
            result = release.audit(folder=self.root, info={"platform": "linux", "architecture": "arm64", "glibc": "2.39"})
        self.assertIn("GLIBCXX_3.4.32", result["ffprobe"]["symbol_versions"])

    def test_failed_publication_restores_previous_complete_release(self):
        destination = self.root / "dist"
        final = self.fixture_release(directory=destination / "releases/0.1.0", value=b"old")
        staged = self.fixture_release(directory=self.root / "staged")
        rename = Path.rename
        def failing_rename(path, target):
            if path == staged:
                raise PermissionError("injected publication failure")
            return rename(path, target=target)
        with mock.patch.object(Path, "rename", new=failing_rename):
            with self.assertRaises(PermissionError):
                release.publish_release(staged=staged, destination=destination, version="0.1.0")
        self.assertEqual((final / "ascii-painter-0.1.0-windows-x64.zip").read_bytes(), b"old")
        release.verify_release_directory(directory=final, version="0.1.0")

    def test_existing_unrecognized_release_files_are_preserved(self):
        destination = self.root / "dist"
        final = destination / "releases/0.1.0"
        final.mkdir(parents=True)
        (final / "personal-file").write_bytes(b"keep")
        staged = self.fixture_release(directory=self.root / "staged")
        with self.assertRaises(release.ReleaseError):
            release.publish_release(staged=staged, destination=destination, version="0.1.0")
        self.assertEqual((final / "personal-file").read_bytes(), b"keep")

    def test_failed_rollback_keeps_the_recovery_backup(self):
        destination = self.root / "dist"
        self.fixture_release(directory=destination / "releases/0.1.0", value=b"old")
        staged = self.fixture_release(directory=self.root / "staged")
        backup = destination / "releases/.previous-0.1.0"
        rename = Path.rename
        def failing_rename(path, target):
            if path in (staged, backup):
                raise PermissionError("injected rename failure")
            return rename(path, target=target)
        with mock.patch.object(Path, "rename", new=failing_rename), mock.patch.object(sys, "stderr", new=io.StringIO()) as errors:
            with self.assertRaises(PermissionError):
                release.publish_release(staged=staged, destination=destination, version="0.1.0")
            self.assertIn(str(backup), errors.getvalue())
        release.verify_release_directory(directory=backup, version="0.1.0")
        self.assertEqual((backup / "ascii-painter-0.1.0-windows-x64.zip").read_bytes(), b"old")

    def test_second_release_lock_fails_instead_of_waiting(self):
        destination = self.root / "dist"
        with release.release_lock(destination):
            with self.assertRaisesRegex(release.ReleaseError, "Another release"):
                with release.release_lock(destination):
                    self.fail("second release acquired the output lock")


if __name__ == "__main__":
    unittest.main()
