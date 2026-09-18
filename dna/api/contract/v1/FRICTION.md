# Native contract validator friction

## Unicode escape decoding differs from the expected string

During this port, a program built with this worktree's `target/release/hale`
printed `?` for `std::json::unescape_string("\\u0031")`; the expected string
was `1`. The contract regression also observed `"hale.v\\u0031"` decoding
to `hale.v?`, which could not match the constant `hale.v1`. This records the
observed behavior, not a diagnosed compiler or runtime cause.

The validator uses a small native decoder in `validator/syntax.hl` after
strict JSON syntax validation. It reads each four-digit escape as a code
point, combines a validated surrogate pair when present, appends its UTF-8
bytes to a let-bound `std::bytes::BytesBuilder`, and returns
`std::str::from_bytes(bytes.snapshot())`. The same decoder handles both
schema and response strings, so escaped constants, enums and local reference
names cannot be interpreted differently between validation stages.

`tests/validator_test.hl` covers ASCII escapes in constants and references,
BMP and surrogate-pair character counts, and escaped/literal astral-string
equality. Escaped U+0000 is rejected before decoding because the native
string operations used here are NUL-terminated; it must not truncate a
constant or enum comparison into an unintended match.
