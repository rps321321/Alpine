def chunk_ranges(total: int, size: int) -> list[tuple[int, int]]:
    """Return contiguous half-open ranges covering ``range(total)``."""
    if total < 0:
        raise ValueError("total must be non-negative")
    if size <= 0:
        raise ValueError("size must be positive")
    return [(start, min(start + size, total)) for start in range(0, total + 1, size)]
