"""Exercise the actual release signer with disposable keys and both archive formats."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile
import package

SIGNER = package.ROOT / 'target/debug/examples' / ('release.exe' if os.name == 'nt' else 'release')


class SigningTests(unittest.TestCase):
    def test_keys_and_both_archive_formats(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            private = root / 'private.key'
            generated = subprocess.run([str(SIGNER), 'keygen', str(private)], check=True, capture_output=True, text=True)
            public = generated.stdout.strip()
            self.assertEqual(len(public), 64)
            original = private.read_bytes()
            if os.name != 'nt':
                self.assertEqual(private.stat().st_mode & 0o777, 0o600)
            self.assertNotEqual(subprocess.run([str(SIGNER), 'keygen', str(private)], capture_output=True).returncode, 0)
            self.assertEqual(private.read_bytes(), original)
            binary = root / 'binary'
            binary.write_bytes(b'example executable')
            env = dict(os.environ, KBMOUSE_UPDATE_SIGNING_KEY=private.read_text().strip(), KBMOUSE_UPDATE_PUBLIC_KEY=public)
            for target in ('x86_64-unknown-linux-gnu', 'x86_64-pc-windows-msvc'):
                archive, = package.package(target, binary, root / 'dist')
                unsigned = archive.read_bytes()
                bad_env = dict(env, KBMOUSE_UPDATE_PUBLIC_KEY='00' * 32)
                bad = subprocess.run([str(SIGNER), 'sign', str(archive)], env=bad_env, capture_output=True)
                self.assertNotEqual(bad.returncode, 0)
                self.assertEqual(archive.read_bytes(), unsigned)
                subprocess.run([str(SIGNER), 'sign', str(archive)], env=env, check=True)
                self.assertNotEqual(archive.read_bytes(), unsigned)
                if target.endswith('windows-msvc'):
                    with zipfile.ZipFile(archive) as zipped:
                        self.assertEqual(zipped.read('kbmouse.exe'), binary.read_bytes())


if __name__ == '__main__':
    unittest.main()
