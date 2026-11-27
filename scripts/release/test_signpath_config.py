import os
import re
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
KEYS = ('SIGNPATH_API_TOKEN', 'SIGNPATH_ORGANIZATION_ID', 'SIGNPATH_SIGNER_SUBJECT')


def windows_job():
    workflow = (ROOT / '.github/workflows/release.yml').read_text()
    return re.search(r'^  windows:\n(.*?)(?=^  \w[\w-]*:\n|\Z)', workflow, re.M | re.S).group(1)


class SignPathConfigurationTests(unittest.TestCase):
    def test_windows_job_covers_build_and_signing_approval_budgets(self):
        windows = windows_job()
        job = int(re.search(r'^    timeout-minutes: (\d+)', windows, re.M).group(1))
        build = re.search(r'- name: Build\n        timeout-minutes: (\d+)', windows)
        signing = re.search(r'- name: Sign the Windows setup\n        timeout-minutes: (\d+)', windows)
        self.assertIsNotNone(build, 'the build needs an explicit bounded share of the job')
        self.assertIsNotNone(signing, 'the signing action needs an explicit bounded share of the job')
        build_minutes, signing_minutes = int(build.group(1)), int(signing.group(1))
        waits = sum(int(re.search(rf'{field}: (\d+)', windows).group(1)) for field in (
            'wait-for-completion-timeout-in-seconds',
            'service-unavailable-timeout-in-seconds',
            'download-signed-artifact-timeout-in-seconds',
        ))
        self.assertGreaterEqual(signing_minutes * 60, waits)
        # Reserve thirty minutes for tool/dependency setup, verification and uploads.
        self.assertGreaterEqual(job, build_minutes + signing_minutes + 30)

    def test_final_updater_signature_binds_version_and_target_after_signpath(self):
        windows = windows_job()
        signer = windows.split('      - name: Sign the final Windows bytes for the updater\n', 1)[1].split('\n      - name:', 1)[0]
        self.assertNotIn('        if:', signer)
        self.assertNotIn('working-directory:', signer)
        self.assertIn('cargo run --locked -p xtask -- sign-windows-updater', signer)
        self.assertIn('"target/release/bundle/nsis/Quasar_${VERSION}_x64-setup.exe" "${VERSION}"', signer)
        self.assertIn('TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}', signer)
        self.assertLess(windows.index('Copy-Item -LiteralPath $signed'), windows.index('Sign the final Windows bytes for the updater'))
        self.assertLess(windows.index('Sign the final Windows bytes for the updater'), windows.index('Stage release assets'))
        self.assertEqual(windows.count('sign-windows-updater'), 1)
        self.assertNotIn('pnpm tauri signer sign', windows)

    def run_config(self, configured):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'output'
            env = {k: v for k, v in os.environ.items() if k not in KEYS}
            env.update(configured, GITHUB_OUTPUT=str(output))
            result = subprocess.run(['bash', str(ROOT / 'scripts/release/signpath_config.sh')],
                                    env=env, capture_output=True, text=True)
            return result.returncode, output.read_text() if output.exists() else '', result.stdout

    def test_absent_configuration_leaves_signing_inactive(self):
        status, output, message = self.run_config({})
        self.assertEqual(status, 0)
        self.assertEqual(output, 'enabled=false\n')
        self.assertIn('unsigned', message)

    def test_partial_configuration_never_silently_ships_unsigned(self):
        for key in KEYS:
            status, output, _ = self.run_config({key: 'fixture'})
            self.assertNotEqual(status, 0)
            self.assertEqual(output, '')

    def test_complete_configuration_activates_without_disclosing_values(self):
        status, output, message = self.run_config({key: 'private-fixture' for key in KEYS})
        self.assertEqual(status, 0)
        self.assertEqual(output, 'enabled=true\n')
        self.assertNotIn('private-fixture', message)


if __name__ == '__main__': unittest.main()
