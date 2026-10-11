#!/usr/bin/env python3
"""Generate notices only; Python 3.11+ is a build-time tool, not an app dependency."""
import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path


def permitted(expression, allowed):
    """Evaluate SPDX AND/OR/WITH, including Cargo's legacy slash-as-OR syntax."""
    tokens = re.findall(r"[A-Za-z0-9.+-]+|[()/]", expression)
    if re.sub(r"\s+", "", expression) != "".join(tokens):
        raise ValueError(f"unsupported license syntax: {expression}")
    pos = 0

    def atom():
        nonlocal pos
        if pos >= len(tokens):
            raise ValueError(f"incomplete license expression: {expression}")
        token = tokens[pos]
        pos += 1
        if token == "(":
            value = combine("OR")
            if pos >= len(tokens) or tokens[pos] != ")":
                raise ValueError(f"unbalanced license expression: {expression}")
            pos += 1
            return value
        if token in {"AND", "OR", "WITH", ")", "/"}:
            raise ValueError(f"unexpected license token: {token}")
        if pos < len(tokens) and tokens[pos] == "WITH":
            pos += 1
            if pos >= len(tokens) or tokens[pos] in {"AND", "OR", "WITH", "(", ")", "/"}:
                raise ValueError(f"missing license exception: {expression}")
            token += " WITH " + tokens[pos]
            pos += 1
        return token in allowed

    def combine(operator):
        nonlocal pos
        value = atom() if operator == "AND" else combine("AND")
        while pos < len(tokens) and tokens[pos] in ({"OR", "/"} if operator == "OR" else {"AND"}):
            pos += 1
            other = atom() if operator == "AND" else combine("AND")
            value = (value and other) if operator == "AND" else (value or other)
        return value

    value = combine("OR")
    if pos != len(tokens):
        raise ValueError(f"unexpected trailing license tokens: {expression}")
    return value


def notice_files(directory):
    # Restrict collection to notice files, never arbitrary dependency source.
    return sorted(p for p in directory.rglob("*") if p.is_file()
                  and re.match(r"^(license|licence|copying|copyright|notice)([._-].*)?$",
                               p.name, re.IGNORECASE)
                  and p.suffix.lower() not in {".rs", ".c", ".h", ".cpp"})


def cell(value):
    return str(value or "Not declared").replace("|", "\\|").replace("\n", " ")


def generate(root, metadata, policy, native):
    with policy.open("rb") as file:
        licenses = tomllib.load(file)["licenses"]
    packages = sorted((p for p in metadata["packages"] if p["id"] not in metadata["workspace_members"]),
                      key=lambda p: (p["name"], p["version"], p["id"]))
    bundle = {"LICENSE": (root / "LICENSE").read_bytes()}
    lines = ["# Third-party notices", "", "Generated from cargo metadata --locked (all resolved platforms, including build dependencies).",
             "No dependency source code is redistributed. OR alternatives require one allowed branch;",
             "AND requires every branch, using deny.toml's allowlist and exact crate exceptions.",
             "License files are copied verbatim per crate: copyright notices are not deduplicated.",
             "Missing files are explicitly noted, not replaced with invented license text.", "",
             "Glance is MIT; see LICENSE. The app icon is original artwork created",
             "for this MIT app, not third-party artwork or an icon set.", "",
             "EXR/HDR, PQ/HLG, XCF and AVIF/JXL sequences are not supported yet.", "",
             f"## Resolved dependencies ({len(packages)} crates)", "",
             "| Crate | Version | License expression | Repository | License/notice files |",
             "| --- | --- | --- | --- | --- |"]
    for package in packages:
        name, version = package["name"], package["version"]
        allowed = set(licenses["allow"])
        for exception in licenses.get("exceptions", []):
            if exception["crate"] == f"{name}@{version}":
                allowed.update(exception["allow"])
        expression = package.get("license") or ""
        if not permitted(expression, allowed):
            raise ValueError(f"disallowed license for {name}@{version}: {expression}")
        source = Path(package["manifest_path"]).parent
        if not (package.get("source") or "").startswith("registry+"):
            raise ValueError(f"not an installed registry crate: {name}@{version}")
        files = notice_files(source)
        declared = package.get("license_file")
        if declared:
            file = (source / declared).resolve()
            if not file.is_relative_to(source.resolve()) or not file.is_file():
                raise ValueError(f"invalid license_file for {name}@{version}")
            files = sorted(set(files + [file]))
        references = []
        for file in files:
            relative = file.resolve().relative_to(source.resolve())
            destination = f"crates/{name}-{version}/{relative.as_posix()}"
            bundle[destination] = file.read_bytes()
            references.append(f"[{relative.as_posix()}]({destination})")
        note = "; ".join(references) or "MISSING: crate archive ships no license/notice file; review upstream before distribution."
        lines.append(f"| {cell(name)} | {cell(version)} | {cell(expression)} | {cell(package.get('repository'))} | {note} |")
    lines += ["", "## Bundled fonts", "", "epaint_default_fonts embeds Hack (MIT/Bitstream), Noto Emoji (OFL-1.1),",
              "Ubuntu Light (Ubuntu-font-1.0), and emoji-icon-font (MIT). Unmodified notices:", ""]
    for file in sorted((root / "licenses/fonts").iterdir()):
        if file.is_file():
            destination = f"fonts/{file.name}"
            bundle[destination] = file.read_bytes()
            lines.append(f"- [{file.name}]({destination})")
    lines += ["", "## Bundled native libraries", "", "libavif, dav1d, and libavif's embedded upstream scaling subset have separate notices.",
              "The native AVIF bundling step records the exact files/versions and upstream attribution;",
              "these must describe the actual packaged libraries, not system development libraries.", ""]
    native_files = sorted(p for p in native.rglob("*") if p.is_file()) if native.is_dir() else []
    if native_files:
        for file in native_files:
            # This input is the bundler's dedicated notices directory, never its source/build tree.
            destination = f"native/{file.relative_to(native).as_posix()}"
            bundle[destination] = file.read_bytes()
            lines.append(f"- [{destination}]({destination})")
        versions = native.parent.parent.parent.parent / "versions.txt"
        if versions.is_file() and native.name == "native":
            bundle["native/versions.txt"] = versions.read_bytes()
            lines.append("- [Exact native versions](native/versions.txt)")
    else:
        lines.append("TODO: native notices are not yet staged. Run the native AVIF bundling step before packaging.")
    bundle["THIRD-PARTY-NOTICES.md"] = ("\n".join(lines) + "\n").encode()
    return bundle, len(packages)


def main():
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=root / "build/notices")
    parser.add_argument("--metadata", type=Path, help="saved cargo metadata JSON (for offline testing)")
    parser.add_argument("--policy", type=Path, default=root / "deny.toml")
    parser.add_argument("--native-notices", type=Path,
                        default=root / "build/bundle-libs/Contents/Resources/licenses/native")
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text()) if args.metadata else json.loads(
        subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=root))
    bundle, count = generate(root, metadata, args.policy, args.native_notices)
    # Refuse stale/caller-owned files rather than recursively deleting an existing directory.
    existing = {p.relative_to(args.output).as_posix() for p in args.output.rglob("*") if p.is_file()}
    if existing - bundle.keys() or args.output.is_symlink() or any(p.is_symlink() for p in args.output.rglob("*")):
        raise ValueError("output contains stale/foreign files or symlinks; choose a fresh --output directory")
    for relative, data in sorted(bundle.items()):
        destination = args.output / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if not destination.exists() or destination.read_bytes() != data:
            destination.write_bytes(data)
    print(f"Notices: {count} crates, {len(bundle)} files, {sum(map(len, bundle.values()))} bytes in {args.output}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        print(f"Notices generation failed: {error}", file=sys.stderr)
        sys.exit(1)
