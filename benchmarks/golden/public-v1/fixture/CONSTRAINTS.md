# Receipt repair constraints

The required wire prefix is exactly `ALPINE-PUBLIC-V1` and must remain the first field.

This fixture is intentionally public. It is a deterministic qualification task, not a hidden task.
All money values are integer cents; floating-point arithmetic is forbidden.
The public functions and their argument order must remain unchanged.
Invalid unit prices and quantities must raise `ValueError`.
The receipt format uses a vertical bar between fields.
The second field is `items=<quantity>`.
The third field is `subtotal=<integer cents>`.
There is no trailing separator and no extra whitespace.
The pricing module owns validation and subtotal calculation.
The rendering module must call the pricing module instead of duplicating arithmetic.
Tests are immutable qualification inputs.
This constraint document is immutable qualification input.
No dependency may be added.
No generated file may be retained.
No recovery sentinel may be created.
The implementation must stay synchronous.
The implementation must stay deterministic.
The implementation must not read environment variables.
The implementation must not use the network.
The implementation must not read the clock.
The implementation must not use randomness.
The implementation must not cache results.
The implementation must not mutate arguments.
The implementation must not add global mutable state.
The implementation must not catch `ValueError` from pricing.
The implementation must not change error types.
The implementation must not add optional parameters.
The implementation must not rename public functions.
The implementation must not add alternate receipt formats.
The implementation must not special-case test values.
The implementation must work for zero quantity.
The implementation must work for a one-cent unit price.
The implementation must work for large integer values supported by Python.
The implementation must keep source files as UTF-8 text.
The implementation must be readable without external context.
The implementation should be the smallest coherent repair.
The agent should run the complete fixture test command.
The final response is not itself qualification evidence.
Only the test result, protected-path hashes, trace counts, and allowlisted file set are evidence.
The task-suite digest binds this document, the prompt, sources, and tests.
The OpenCode executable digest binds the engine used for a live run.
The Alpine executable digest is bound by the external-evidence envelope.
An unexpected file makes the task fail.
A protected-file modification makes the task fail.
A missing expected tool failure makes the task fail.
More than three tool failures makes the task fail.
Passing only one source repair makes the task fail.
Forgetting the first wire field makes the task fail.
Repeating the wire prefix makes the task fail.
Changing field order makes the task fail.
Returning a non-string receipt makes the task fail.
Returning a non-integer subtotal makes the task fail.
These repeated boundaries make context grow while the first exact constraint remains authoritative.
