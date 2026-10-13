#!/usr/bin/env bash
# Usage: bash scripts/package-release.sh TAG TARGET BINARY OUTPUT_DIR [Cargo.toml]
set -euo pipefail
python3 - "$@" <<'PY'
import hashlib
import os
from pathlib import Path
import platform
import re
import sys
import tarfile
import tempfile
import tomllib

try:
    if len(sys.argv) not in (5, 6):
        raise ValueError('usage: package-release.sh TAG TARGET BINARY OUTPUT_DIR [Cargo.toml]')
    tag, target, source, destination = sys.argv[1:5]
    if not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag):
        raise ValueError('tag must be vMAJOR.MINOR.PATCH')
    manifest = Path(sys.argv[5] if len(sys.argv) == 6 else 'Cargo.toml')
    if tomllib.loads(manifest.read_text())['package']['version'] != tag[1:]:
        raise ValueError('tag does not match crate version')
    arch = {'arm64': 'aarch64', 'aarch64': 'aarch64', 'x86_64': 'x86_64'}.get(platform.machine())
    system = {'Darwin': 'apple-darwin', 'Linux': 'unknown-linux-gnu'}.get(platform.system())
    if not arch or not system or target != f'{arch}-{system}':
        raise ValueError('target must match native host')
    binary = Path(source)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('binary must be an executable file')
    output = Path(destination)
    name = f'obm-{tag[1:]}-{target}.tar.gz'
    if (output / name).exists() or (output / (name + '.sha256')).exists():
        raise ValueError('output artifacts already exist')
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output) as temporary:
        archive = Path(temporary) / name
        with tarfile.open(archive, 'w:gz') as packed:
            packed.add(binary, arcname='obm', recursive=False)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        checksum = Path(temporary) / (name + '.sha256')
        checksum.write_text(f'{digest}  {name}\n')
        archive.rename(output / name)
        checksum.rename(output / checksum.name)
except (ValueError, OSError, KeyError) as error:
    print(f'error: {error}', file=sys.stderr)
    sys.exit(1)
PY
