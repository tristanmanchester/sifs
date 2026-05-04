#[cfg(target_os = "macos")]
pub fn peak_rss_mb() -> f64 {
    peak_rss_raw() as f64 / (1024.0 * 1024.0)
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn peak_rss_mb() -> f64 {
    peak_rss_raw() as f64 / 1024.0
}

#[cfg(unix)]
fn peak_rss_raw() -> libc::c_long {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage writes a complete rusage value to the provided pointer
    // when it returns 0, so assume_init is only called after that success code.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc == 0 {
        unsafe { usage.assume_init() }.ru_maxrss
    } else {
        0
    }
}

#[cfg(not(unix))]
pub fn peak_rss_mb() -> f64 {
    0.0
}
