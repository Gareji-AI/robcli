"""Pack one approved robcli archive. Checksums are not a publisher signature."""

import argparse
import hashlib
import pathlib
import tarfile
import zipfile

APPROVED = {
    ("windows", "x86_64"): ("rob.exe", "zip"),
    ("linux", "x86_64"): ("rob", "tar.gz"),
    ("macos", "aarch64"): ("rob", "tar.gz"),
    ("macos", "x86_64"): ("rob", "tar.gz"),
}
LICENSES = ("license-mit", "license-apache")


def plain_version(version: str) -> bool:
    return (
        bool(version)
        and all(char.isdigit() or char == "." for char in version)
        and not version.startswith(".")
        and not version.endswith(".")
        and ".." not in version
    )


def archive_name(version: str, system: str, arch: str) -> tuple[str, str]:
    if not plain_version(version):
        raise ValueError("version is not a plain numeric release")
    binary, extension = APPROVED[(system, arch)]
    return f"robcli-{version}-{system}-{arch}.{extension}", binary


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--os", required=True, dest="system")
    parser.add_argument("--arch", required=True)
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--licenses", required=True, type=pathlib.Path)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    args = parser.parse_args()
    try:
        name, binary_name = archive_name(args.version, args.system, args.arch)
    except KeyError:
        raise SystemExit(
            f"target {args.system}/{args.arch} is outside the approved archive matrix"
        ) from None
    except ValueError as error:
        raise SystemExit(str(error)) from error
    if args.binary.name != binary_name or not args.binary.is_file():
        raise SystemExit("binary name is outside the approved archive member list")
    licenses = [args.licenses / item for item in LICENSES]
    if any(not path.is_file() for path in licenses):
        raise SystemExit("license files are missing")
    args.out.mkdir(parents=True, exist_ok=True)
    destination = args.out / name
    if destination.exists():
        raise SystemExit("archive already exists; refusing to replace it")
    members = [args.binary, *licenses]
    if name.endswith(".zip"):
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for path in members:
                archive.write(path, arcname=path.name)
    else:
        with tarfile.open(destination, "w:gz") as archive:
            for path in members:
                archive.add(path, arcname=path.name)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    checksum = args.out / f"{name}.sha256"
    # Pin the newline so the file stays `sha256sum -c` compatible on every host.
    with checksum.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(f"{digest}  {name}\n")
    print(checksum)


if __name__ == "__main__":
    main()
