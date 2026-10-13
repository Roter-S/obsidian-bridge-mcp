#!/usr/bin/env python3
"""Render a binary formula from four real archives and matching SHA256 sidecars."""
import hashlib
from pathlib import Path
import re
import sys

TARGETS = ('aarch64-apple-darwin', 'x86_64-apple-darwin',
           'aarch64-unknown-linux-gnu', 'x86_64-unknown-linux-gnu')


def main():
    if len(sys.argv) != 4:
        raise ValueError('usage: render-formula.py VERSION ARTIFACT_DIR OUTPUT.rb')
    version, directory, output = sys.argv[1:]
    if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', version):
        raise ValueError('version must be MAJOR.MINOR.PATCH')
    hashes = {}
    for target in TARGETS:
        name = f'obm-{version}-{target}.tar.gz'
        archive = Path(directory) / name
        checksum = Path(directory) / (name + '.sha256')
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        if checksum.read_text() != f'{digest}  {name}\n':
            raise ValueError(f'checksum mismatch: {name}')
        hashes[target] = digest
    lines = ['class ObsidianBridgeMcp < Formula', '  desc "Obsidian MCP bridge"',
             '  homepage "https://github.com/Roter-S/obsidian-bridge-mcp"',
             f'  version "{version}"']
    for system, suffix in [('macos', 'apple-darwin'), ('linux', 'unknown-linux-gnu')]:
        lines.append(f'  on_{system} do')
        for cpu, arch in [('arm', 'aarch64'), ('intel', 'x86_64')]:
            target = f'{arch}-{suffix}'
            lines.extend([f'    on_{cpu} do',
                          f'      url "https://github.com/Roter-S/obsidian-bridge-mcp/releases/download/v{version}/obm-{version}-{target}.tar.gz"',
                          f'      sha256 "{hashes[target]}"', '    end'])
        lines.append('  end')
    lines.extend(['', '  def install', '    bin.install "obm"', '  end', 'end', ''])
    # All inputs are validated before touching the destination.
    Path(output).write_text('\n'.join(lines))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError) as error:
        print(f'error: {error}', file=sys.stderr)
        sys.exit(1)
