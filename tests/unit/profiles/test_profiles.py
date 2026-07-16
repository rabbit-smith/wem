import hashlib
import unittest

from wwise_wem import PROFILES, ProfileKey, resolve_wem_profile


class ProfileTests(unittest.TestCase):
    def test_installed_profile(self):
        profile = resolve_wem_profile(6, 44100)
        self.assertEqual(profile.name, "wwise2013-6ch-44100")
        self.assertEqual(len(profile.setup_packet()), 201)
        self.assertEqual(
            hashlib.sha256(profile.setup_packet()).hexdigest(),
            "3ef56cbd6e6a66a5474005db05912624487faa555fb2cdfed130f606b322e4e3",
        )

    def test_registry_key(self):
        self.assertEqual(len(PROFILES), 1)
        self.assertEqual(ProfileKey(6, 44100).channels, 6)

    def test_unknown_geometry(self):
        with self.assertRaisesRegex(ValueError, "no Wwise 2013.2 profile"):
            resolve_wem_profile(2, 48000)


if __name__ == "__main__":
    unittest.main()
