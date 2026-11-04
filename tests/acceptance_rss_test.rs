//! Exercise the acceptance helper's failure paths without heavy fixtures.
#![cfg(target_os = "linux")]
mod acceptance;

#[test]
fn rss_acceptance_rejects_missing_or_over_budget_measurements() {
    for (reset, before, peak, bound) in [
        (false, 1, 2, 1024),
        (true, 0, 2, 1024),
        (true, 1, 0, 1024),
        (true, 1, 3, 1024),
    ] {
        assert!(
            std::panic::catch_unwind(|| acceptance::check_rss("probe", reset, before, peak, bound))
                .is_err()
        );
    }
}

#[test]
fn rss_acceptance_measures_at_the_bound() {
    acceptance::check_rss("probe", true, 1, 2, 1024);
}
