use omron_mcp::cli::{self, Settings};

// The static musl build links Zig's libc malloc, which retains freed small allocations in
// per-CPU, per-size-class freelists and never unmaps those slabs: after a burst the RSS ranged
// from 9 to 14 MB over a 0.86 MB live heap (measured in mfp-mcp-rs).
#[cfg(not(feature = "testhooks"))]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

unsafe extern "C" {
    fn mi_option_set(option: i32, value: std::ffi::c_long);
    #[cfg(test)]
    fn mi_option_get(option: i32) -> std::ffi::c_long;
}

// The index of `mi_option_purge_delay` in mimalloc v3's option enum (libmimalloc-sys has no
// constant for it); the test below fails if an upgrade moves it.
const MI_OPTION_PURGE_DELAY: i32 = 15;

// An idle process makes no allocator calls, so a delayed purge would never run and the memory
// freed by a burst would stay resident: purge at once. The `no_thp` feature only stops mimalloc
// asking for huge pages; on a host with THP set to `always` the kernel would still back its
// arena with 2 MB pages, so turn THP off for the process (what mimalloc's allow_thp=0 does).
// That is best effort: a kernel without THP refuses it harmlessly, so a failure is reported but
// not fatal.
fn configure_allocator() {
    let thp_disabled = unsafe {
        mi_option_set(MI_OPTION_PURGE_DELAY, 0);

        // musl's wrapper reads each variadic argument as an unsigned long.
        libc::prctl(
            libc::PR_SET_THP_DISABLE,
            1 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        )
    };

    if thp_disabled != 0 {
        eprintln!(
            "omron-mcp: could not turn off transparent huge pages: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(not(feature = "testhooks"))]
fn main() {
    configure_allocator();

    mcp_starter::runtime::run_main(|args, io, getenv| {
        cli::run(args, io, getenv, Settings::production())
    })
}

#[cfg(feature = "testhooks")]
fn main() {
    configure_allocator();

    mcp_starter::runtime::run_main(|args, io, getenv| {
        let report_heap = args.first().is_none_or(|command| command == "serve");

        async move {
            let outcome = cli::run(args, io, getenv, Settings::production()).await;

            if report_heap {
                eprintln!(
                    "heap peak_bytes={} live_bytes={}",
                    testhooks::peak_bytes(),
                    testhooks::live_bytes()
                );
            }

            outcome
        }
    })
}

#[cfg(feature = "testhooks")]
#[global_allocator]
static ALLOCATOR: testhooks::CountingAllocator = testhooks::CountingAllocator;

#[cfg(feature = "testhooks")]
mod testhooks {
    use std::alloc::{GlobalAlloc, Layout};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mimalloc::MiMalloc;

    static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
    static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

    pub struct CountingAllocator;

    impl CountingAllocator {
        fn allocated(size: usize) {
            let live = LIVE_BYTES.fetch_add(size, Ordering::Relaxed) + size;
            let mut peak = PEAK_BYTES.load(Ordering::Relaxed);

            while live > peak {
                match PEAK_BYTES.compare_exchange_weak(
                    peak,
                    live,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(current) => peak = current,
                }
            }
        }

        fn resized(old_size: usize, new_size: usize) {
            if new_size > old_size {
                Self::allocated(new_size - old_size);
            } else if old_size > new_size {
                LIVE_BYTES.fetch_sub(old_size - new_size, Ordering::Relaxed);
            }
        }
    }

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { MiMalloc.alloc(layout) };

            if !pointer.is_null() {
                Self::allocated(layout.size());
            }

            pointer
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let pointer = unsafe { MiMalloc.alloc_zeroed(layout) };

            if !pointer.is_null() {
                Self::allocated(layout.size());
            }

            pointer
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { MiMalloc.dealloc(pointer, layout) };

            LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let resized = unsafe { MiMalloc.realloc(pointer, layout, new_size) };

            if !resized.is_null() {
                Self::resized(layout.size(), new_size);
            }

            resized
        }
    }

    pub fn live_bytes() -> usize {
        LIVE_BYTES.load(Ordering::Relaxed)
    }

    pub fn peak_bytes() -> usize {
        PEAK_BYTES.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1000 ms is purge_delay's v3 default and no other option's (options.c in libmimalloc-sys).
    // mimalloc reads an override from either variable at start-up, matching the name in any
    // letter case and with its own parsing rules, so with one set the default cannot be checked.
    #[test]
    fn the_purge_delay_index_names_purge_delay() {
        let overridden = std::env::vars_os().any(|(name, _)| {
            ["MIMALLOC_PURGE_DELAY", "MIMALLOC_RESET_DELAY"]
                .iter()
                .any(|alias| name.eq_ignore_ascii_case(alias))
        });

        if overridden {
            return;
        }

        assert_eq!(unsafe { mi_option_get(MI_OPTION_PURGE_DELAY) }, 1000);
    }
}
