"""Package native release builds; run on Windows x64 or macOS Apple Silicon."""

import argparse
import hashlib
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import struct
import subprocess
import tempfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-pc-windows-msvc": ("Windows", "windows-x64", "AIDRAW.exe"),
    "aarch64-apple-darwin": ("Darwin", "macos-arm64", "AIDRAW"),
}
MACOS_MINIMUM = "13.0"


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def copy_documents(destination):
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "README.md", destination / "README.md")
    licenses = destination / "licenses"
    licenses.mkdir()
    shutil.copy2(
        ROOT / "assets/fonts/LICENSE-Maple-Mono.txt",
        licenses / "LICENSE-Maple-Mono.txt",
    )


def windows_package(binary, staging, output, stem):
    # Fail if an x86/ARM executable is accidentally passed to the x64 job.
    with binary.open("rb") as source:
        if source.read(2) != b"MZ":
            raise ValueError("Not a Windows executable")
        source.seek(0x3C)
        pe_offset = struct.unpack("<I", source.read(4))[0]
        source.seek(pe_offset)
        if source.read(6) != b"PE\0\0\x64\x86":
            raise ValueError("Expected a Windows x64 (AMD64) executable")
    folder = staging / "AIDRAW"
    copy_documents(folder)
    shutil.copy2(binary, folder / "AIDRAW.exe")
    archive = output / f"{stem}.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as package:
        for path in sorted(folder.rglob("*")):
            if path.is_file():
                package.write(path, path.relative_to(staging).as_posix())
    return [archive]


def macos_package(binary, version, staging, output, stem):
    architectures = subprocess.check_output(
        ["lipo", "-archs", str(binary)], text=True
    ).strip()
    if architectures != "arm64":
        raise ValueError(f"Expected an ARM64-only executable, got {architectures}")
    app = staging / "AIDRAW.app"
    contents = app / "Contents"
    executable = contents / "MacOS/AIDRAW"
    executable.parent.mkdir(parents=True)
    shutil.copy2(binary, executable)
    executable.chmod(0o755)
    resources = contents / "Resources"
    copy_documents(resources)
    # sips/iconutil ship with macOS; packaging needs no pip dependencies.
    with tempfile.TemporaryDirectory(prefix="aidraw-icons-") as icon_temp:
        iconset = Path(icon_temp) / "AIDRAW.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                pixels = str(size * scale)
                suffix = "@2x" if scale == 2 else ""
                run(
                    "sips", "-z", pixels, pixels,
                    ROOT / "assets/icons/aidraw.png", "--out",
                    iconset / f"icon_{size}x{size}{suffix}.png",
                )
        run("iconutil", "-c", "icns", iconset, "-o", resources / "AIDRAW.icns")
    numeric_version = re.match(r"\d+\.\d+\.\d+", version).group()
    metadata = {
        "CFBundleDevelopmentRegion": "zh_CN",
        "CFBundleIdentifier": "com.aidraw.studio",
        "CFBundleName": "AIDRAW",
        "CFBundleDisplayName": "三只猫专用图像工作室",
        "CFBundleExecutable": "AIDRAW",
        "CFBundleIconFile": "AIDRAW.icns",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": numeric_version,
        "CFBundleVersion": numeric_version,
        "LSMinimumSystemVersion": MACOS_MINIMUM,
        "NSHighResolutionCapable": True,
        "NSPrincipalClass": "NSApplication",
    }
    with (contents / "Info.plist").open("wb") as destination:
        plistlib.dump(metadata, destination)
    (contents / "PkgInfo").write_bytes(b"APPL????")
    # Ad-hoc signing is required for native ARM64 bundles. This is not
    # Developer ID signing/notarization and needs no private certificates.
    run("codesign", "--force", "--sign", "-", "--timestamp=none", app)
    run("codesign", "--verify", "--strict", "--verbose=2", app)
    archive = output / f"{stem}.zip"
    run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive)
    (staging / "Applications").symlink_to("/Applications", target_is_directory=True)
    dmg = output / f"{stem}.dmg"
    run(
        "hdiutil", "create", "-volname", "AIDRAW", "-srcfolder", staging,
        "-fs", "HFS+", "-format", "UDZO", "-ov", dmg,
    )
    run("hdiutil", "verify", dmg)
    return [archive, dmg]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    args = parser.parse_args()
    host, label, binary_name = TARGETS[args.target]
    if platform.system() != host:
        parser.error(f"Package {args.target} on {host}, not {platform.system()}")
    if host == "Darwin" and os.environ.get(
        "MACOSX_DEPLOYMENT_TARGET", MACOS_MINIMUM
    ) != MACOS_MINIMUM:
        parser.error(f"Build with MACOSX_DEPLOYMENT_TARGET={MACOS_MINIMUM}")
    with (ROOT / "Cargo.toml").open("rb") as source:
        version = tomllib.load(source)["package"]["version"]
    binary = ROOT / "target" / args.target / "release" / binary_name
    if not binary.is_file():
        parser.error(f"Build the release executable first: {binary}")
    output = ROOT / "dist"
    output.mkdir(exist_ok=True)
    stem = f"AIDRAW-{version}-{label}"
    with tempfile.TemporaryDirectory(prefix="aidraw-package-") as temp:
        staging = Path(temp)
        if host == "Windows":
            packages = windows_package(binary, staging, output, stem)
        else:
            packages = macos_package(binary, version, staging, output, stem)
    checksums = output / f"SHA256SUMS-{label}.txt"
    lines = []
    for path in packages:
        with path.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        lines.append(f"{digest}  {path.name}\n")
    checksums.write_text("".join(lines), encoding="utf-8")
    for path in [*packages, checksums]:
        print(f"Created {path}")


if __name__ == "__main__":
    main()
