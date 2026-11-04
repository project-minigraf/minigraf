//! Shared acceptance-only RSS assertion. Linux must measure; other targets say
//! explicitly that they skipped. Run measured tests with --test-threads=1.

pub fn check_rss(label: &str, reset: bool, before_kb: usize, peak_kb: usize, bound: usize) {
    if cfg!(target_os = "linux") {
        assert!(reset, "Linux acceptance must reset /proc/self/clear_refs");
        assert!(before_kb > 0, "Linux acceptance must read VmRSS");
        assert!(peak_kb > 0, "Linux acceptance must read VmHWM");
        let added = peak_kb.saturating_sub(before_kb) * 1024;
        assert!(added <= bound, "acceptance RSS exceeded its bound");
        eprintln!("RSS_CHECK {label} measured added_bytes={added} bound_bytes={bound}");
    } else {
        eprintln!(
            "RSS_CHECK {label} skipped platform={}",
            std::env::consts::OS
        );
    }
}
