//! The `xseed` integration-test binary: 16 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "cross_seed_arity.rs"]
mod cross_seed_arity;
#[path = "cross_seed_effects.rs"]
mod cross_seed_effects;
#[path = "cross_seed_enum_persp.rs"]
mod cross_seed_enum_persp;
#[path = "cross_seed_interface_fallible.rs"]
mod cross_seed_interface_fallible;
#[path = "cross_seed_qualified_serves.rs"]
mod cross_seed_qualified_serves;
#[path = "face_seeds_check.rs"]
mod face_seeds_check;
#[path = "form_hashmap_qualified.rs"]
mod form_hashmap_qualified;
#[path = "hot_factory_xseed.rs"]
mod hot_factory_xseed;
#[path = "user_effects_cross_bus.rs"]
mod user_effects_cross_bus;
#[path = "xseed_claims.rs"]
mod xseed_claims;
#[path = "xseed_claims_verbs.rs"]
mod xseed_claims_verbs;
#[path = "xseed_library_claims.rs"]
mod xseed_library_claims;
#[path = "xseed_obs_counters.rs"]
mod xseed_obs_counters;
#[path = "xseed_publish_alias.rs"]
mod xseed_publish_alias;
#[path = "xseed_topic_identity.rs"]
mod xseed_topic_identity;
#[path = "xseed_user_effects.rs"]
mod xseed_user_effects;
