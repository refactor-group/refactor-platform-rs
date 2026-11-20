use super::*;

#[test]
fn main_pool_bounds_reserves_the_session_pool() {
    assert_eq!(main_pool_bounds(15, 5), (12, 5));
}

#[test]
fn main_pool_bounds_clamps_a_tiny_budget_to_one() {
    assert_eq!(main_pool_bounds(2, 5), (1, 1));
}

#[test]
fn check_search_path_accepts_the_schema_among_others() {
    assert!(check_search_path("refactor_platform,public").is_ok());
}

#[test]
fn check_search_path_rejects_a_path_without_the_schema() {
    assert!(check_search_path("public").is_err());
}

#[test]
fn check_search_path_rejects_a_schema_that_only_shares_its_prefix() {
    assert!(check_search_path("refactor_platform_archive,public").is_err());
}
