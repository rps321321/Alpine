def subtotal_cents(unit_cents: int, quantity: int) -> int:
    """Return the subtotal for an integer-cent unit price."""
    if unit_cents < 0:
        raise ValueError("unit_cents must be non-negative")
    if quantity < 0:
        raise ValueError("quantity must be non-negative")
    return unit_cents + quantity
