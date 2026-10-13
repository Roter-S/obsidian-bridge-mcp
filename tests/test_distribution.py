import hashlib
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ('aarch64-apple-darwin', 'x86_64-apple-darwin',
           'aarch64-unknown-linux-gnu', 'x86_64-unknown-linux-gnu')


class DistributionTests(unittest.TestCase):
    def run_command(self, *args):
        return subprocess.run(args, cwd=ROOT, capture_output=True, text=True)

    def test_formula_real_archives_and_rejection(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            output = directory / 'obsidian-bridge-mcp.rb'
            hashes = []
            for target in TARGETS:
                archive = directory / f'obm-0.1.0-{target}.tar.gz'
                archive.write_bytes(target.encode())
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                hashes.append(digest)
                archive.with_suffix('.gz.sha256').write_text(f'{digest}  {archive.name}\n')
            command = ('python3', str(ROOT / 'scripts/render-formula.py'),
                       '0.1.0', str(directory), str(output))
            result = self.run_command(*command)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, '', ''))
            formula = output.read_text()
            for target, digest in zip(TARGETS, hashes):
                self.assertIn(f'/releases/download/v0.1.0/obm-0.1.0-{target}.tar.gz', formula)
                self.assertIn(f'sha256 "{digest}"', formula)
            self.assertEqual(formula.splitlines()[0], 'class ObsidianBridgeMcp < Formula')
            self.assertIn('  def install\n    bin.install "obm"\n  end\n', formula)
            self.assertEqual(formula.count('bin.install'), 1)
            self.assertNotIn('setup', formula)
            self.assertNotIn('post_install', formula)
            self.assertNotIn('system ', formula)
            original = output.read_bytes()
            (directory / 'obm-0.1.0-aarch64-apple-darwin.tar.gz').write_bytes(b'changed')
            result = self.run_command(*command)
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (1, '', 'error: checksum mismatch: obm-0.1.0-aarch64-apple-darwin.tar.gz\n'))
            self.assertEqual(output.read_bytes(), original)
            output.unlink()
            result = self.run_command(*command)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, '')
            self.assertFalse(output.exists())

    def test_invalid_formula_version_has_no_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'obsidian-bridge-mcp.rb'
            result = self.run_command('python3', str(ROOT / 'scripts/render-formula.py'),
                                      '../bad', tmp, str(output))
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (1, '', 'error: version must be MAJOR.MINOR.PATCH\n'))
            self.assertFalse(output.exists())

    def test_native_packaging_and_version_rejection(self):
        system = {'Darwin': 'apple-darwin', 'Linux': 'unknown-linux-gnu'}[platform.system()]
        arch = {'arm64': 'aarch64', 'aarch64': 'aarch64', 'x86_64': 'x86_64'}[platform.machine()]
        target = f'{arch}-{system}'
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            binary = directory / 'obm'
            binary.write_text('#!/bin/sh\nprintf "fixture\\n"\n')
            binary.chmod(0o755)
            manifest = directory / 'Cargo.toml'
            manifest.write_text('[package]\nversion = "0.1.0"\n')
            output = directory / 'dist'
            command = ('bash', str(ROOT / 'scripts/package-release.sh'))
            result = self.run_command(*command, 'v0.2.0', target, str(binary), str(output), str(manifest))
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (1, '', 'error: tag does not match crate version\n'))
            self.assertFalse(output.exists())
            result = self.run_command(*command, 'v0.1.0', target, str(binary), str(output), str(manifest))
            self.assertEqual((result.returncode, result.stdout, result.stderr), (0, '', ''))
            archive = output / f'obm-0.1.0-{target}.tar.gz'
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            self.assertEqual(archive.with_suffix('.gz.sha256').read_text(), f'{digest}  {archive.name}\n')
            import tarfile
            with tarfile.open(archive) as packaged:
                self.assertEqual(packaged.getnames(), ['obm'])
                self.assertEqual(packaged.extractfile('obm').read(), binary.read_bytes())
                self.assertTrue(packaged.getmember('obm').mode & 0o111)
            other = TARGETS[1] if target != TARGETS[1] else TARGETS[0]
            rejected = directory / 'rejected'
            result = self.run_command(*command, 'v0.1.0', other, str(binary), str(rejected), str(manifest))
            self.assertEqual((result.returncode, result.stdout, result.stderr),
                             (1, '', 'error: target must match native host\n'))
            self.assertFalse(rejected.exists())
