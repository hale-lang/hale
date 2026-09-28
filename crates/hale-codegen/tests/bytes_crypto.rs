//! The `bytes_crypto` integration-test binary: 15 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "base64url.rs"]
mod base64url;
#[path = "bytes_builder.rs"]
mod bytes_builder;
#[path = "bytes_builder_inplace.rs"]
mod bytes_builder_inplace;
#[path = "bytes_builder_view.rs"]
mod bytes_builder_view;
#[path = "bytes_builder_violate.rs"]
mod bytes_builder_violate;
#[path = "bytes_construction.rs"]
mod bytes_construction;
#[path = "bytes_pack_write.rs"]
mod bytes_pack_write;
#[path = "bytes_view_stale.rs"]
mod bytes_view_stale;
#[path = "bytes_view_type.rs"]
mod bytes_view_type;
#[path = "bytes_xor_find.rs"]
mod bytes_xor_find;
#[path = "crypto_crc32.rs"]
mod crypto_crc32;
#[path = "crypto_sha256_hmac.rs"]
mod crypto_sha256_hmac;
#[path = "crypto_sha512_hmac.rs"]
mod crypto_sha512_hmac;
#[path = "ecdsa_p256.rs"]
mod ecdsa_p256;
#[path = "sha1_base64.rs"]
mod sha1_base64;
