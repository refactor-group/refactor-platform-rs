use super::*;

#[test]
fn main_pool_bounds_reserves_the_session_pool() {
    assert_eq!(main_pool_bounds(15, 5), (12, 5));
}

#[test]
fn main_pool_bounds_clamps_a_tiny_budget_to_one() {
    assert_eq!(main_pool_bounds(2, 5), (1, 1));
}
