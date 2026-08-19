import unittest

from src.ranges import chunk_ranges


class ChunkRangesTests(unittest.TestCase):
    def test_divisible_total_has_no_empty_tail(self) -> None:
        self.assertEqual(chunk_ranges(6, 3), [(0, 3), (3, 6)])

    def test_partial_tail_is_clamped(self) -> None:
        self.assertEqual(chunk_ranges(7, 3), [(0, 3), (3, 6), (6, 7)])

    def test_zero_total_is_empty(self) -> None:
        self.assertEqual(chunk_ranges(0, 3), [])

    def test_invalid_arguments(self) -> None:
        with self.assertRaises(ValueError):
            chunk_ranges(-1, 3)
        with self.assertRaises(ValueError):
            chunk_ranges(3, 0)


if __name__ == "__main__":
    unittest.main()
