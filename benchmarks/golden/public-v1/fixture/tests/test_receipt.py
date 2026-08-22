import unittest

from src.pricing import subtotal_cents
from src.rendering import render_receipt


class PublicReceiptQualificationTests(unittest.TestCase):
    def test_pricing_repairs_integer_cent_multiplication(self) -> None:
        self.assertEqual(subtotal_cents(125, 3), 375)
        self.assertEqual(subtotal_cents(1, 0), 0)

    def test_pricing_preserves_validation(self) -> None:
        with self.assertRaises(ValueError):
            subtotal_cents(-1, 2)
        with self.assertRaises(ValueError):
            subtotal_cents(10, -1)

    def test_rendering_retains_the_early_exact_constraint(self) -> None:
        self.assertEqual(
            render_receipt(125, 3),
            "ALPINE-PUBLIC-V1|items=3|subtotal=375",
        )

    def test_rendering_propagates_pricing_validation(self) -> None:
        with self.assertRaises(ValueError):
            render_receipt(-1, 2)


if __name__ == "__main__":
    unittest.main()
