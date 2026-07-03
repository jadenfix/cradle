import unittest

from beatbox.client import _encode_job_id


class TestJobIdEncoding(unittest.TestCase):
    def test_rejects_empty(self):
        with self.assertRaises(ValueError):
            _encode_job_id("")

    def test_rejects_dot(self):
        with self.assertRaises(ValueError):
            _encode_job_id(".")

    def test_rejects_dotdot(self):
        with self.assertRaises(ValueError):
            _encode_job_id("..")

    def test_encodes_path_traversal(self):
        # A traversal attempt must not stay as a slash-separated path.
        self.assertEqual(_encode_job_id("../execute"), "..%2Fexecute")

    def test_encodes_query_like_id(self):
        # '?' and '&' and '=' must be percent-encoded, not treated as a query.
        self.assertEqual(_encode_job_id("x?k=v"), "x%3Fk%3Dv")

    def test_encodes_slash(self):
        self.assertEqual(_encode_job_id("a/b"), "a%2Fb")

    def test_plain_uuid_unchanged(self):
        uuid = "550e8400-e29b-41d4-a716-446655440000"
        self.assertEqual(_encode_job_id(uuid), uuid)


if __name__ == "__main__":
    unittest.main()
