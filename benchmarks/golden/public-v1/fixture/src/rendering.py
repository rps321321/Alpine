from src.pricing import subtotal_cents


def render_receipt(unit_cents: int, quantity: int) -> str:
    """Render the public receipt wire format."""
    subtotal = subtotal_cents(unit_cents, quantity)
    return f"RECEIPT items={quantity} subtotal={subtotal}"
