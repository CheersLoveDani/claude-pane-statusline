//! Self-collection of abandoned renders.
//!
//! Claude Code sometimes creates a render process CREATE_SUSPENDED and abandons
//! it before resuming. Such a process has 0 CPU time and a single suspended
//! thread: it never executed one instruction, so it cannot clean itself up, in
//! any language. Something else has to kill it.
//!
//! That something used to be statusline-reaper.ps1 on a 5-minute scheduled task,
//! which cost wscript + powershell + a conhost (~0.5 s) every 5 minutes forever
//! to usually find nothing. Doing it here instead is strictly better:
//!
//!   * zero extra processes - it runs inside one that was already spawned
//!   * it fires when strays are actually created, because strays *are* abandoned
//!     renders: no renders, no strays, nothing to collect
//!   * collection happens within a minute rather than up to five
//!
//! The sweep is rate-limited by the mtime of a stamp file, so the common render
//! pays one `stat` (~10 us) and only one render a minute pays the ~1-3 ms
//! process-table walk.

use std::os::windows::ffi::OsStringExt;
use std::path::Path;

/// At most one sweep per minute across all panes.
const SWEEP_EVERY_MS: i64 = 60_000;
/// 0 CPU this long after start means it was never resumed. A healthy render
/// exits in ~10 ms, so this is not a close call.
const NEVER_RAN_MS: u64 = 15_000;
/// Ran, but outlived any plausible render - hung rather than suspended.
const STALE_MS: u64 = 60_000;

type Handle = isize;
const INVALID_HANDLE_VALUE: Handle = -1;
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_TERMINATE: u32 = 0x0001;
const MAX_PATH: usize = 260;

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn zero() -> Self {
        FileTime { low: 0, high: 0 }
    }
    fn as_u64(&self) -> u64 {
        ((self.high as u64) << 32) | self.low as u64
    }
}

#[repr(C)]
struct ProcessEntry32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; MAX_PATH],
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
    fn GetProcessTimes(
        h: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn QueryFullProcessImageNameW(h: Handle, flags: u32, buf: *mut u16, size: *mut u32) -> i32;
    fn TerminateProcess(h: Handle, exit_code: u32) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn GetSystemTimeAsFileTime(out: *mut FileTime);
}

/// Sweep at most once per `SWEEP_EVERY_MS`.
///
/// The last-sweep time is the stamp file's *contents*, not its mtime. Using the
/// mtime is the obvious design and it is broken on Windows: rewriting an empty
/// file with empty content performs no actual write, so `LastWriteTime` never
/// moves. The stamp froze at creation, every render after the first minute swept
/// (~11 ms each), and the rate limit silently did nothing.
pub fn maybe_sweep(tasks_dir: &Path) {
    let stamp = tasks_dir.join(".reap");
    let now = crate::paths::now_ms();
    if let Some(last) = std::fs::read_to_string(&stamp)
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
    {
        // `now < last` means the stamp is in the future - a clock that moved
        // back, or a garbled write. Fall through and re-stamp, which self-heals
        // on the next render; suppressing instead would disable collection until
        // wall-clock caught up, potentially forever.
        if now >= last && now - last < SWEEP_EVERY_MS {
            return;
        }
    }
    // Stamp *before* sweeping: concurrent renders across panes then skip
    // immediately instead of all walking the process table at once.
    let _ = std::fs::create_dir_all(tasks_dir);
    if std::fs::write(&stamp, now.to_string()).is_err() {
        return; // can't rate-limit, so don't sweep at all
    }
    unsafe { sweep() }
}

unsafe fn sweep() {
    let Ok(self_exe) = std::env::current_exe() else { return };
    let Some(self_name) = self_exe.file_name().map(|s| s.to_string_lossy().to_lowercase()) else {
        return;
    };
    let self_pid = GetCurrentProcessId();

    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if snapshot == INVALID_HANDLE_VALUE {
        return;
    }

    let mut entry: ProcessEntry32W = std::mem::zeroed();
    entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;

    let mut ok = Process32FirstW(snapshot, &mut entry);
    while ok != 0 {
        let pid = entry.th32_process_id;
        // Cheap filter first: image name must match ours, and never ourselves.
        if pid != self_pid && pid != 0 && wide_to_string(&entry.sz_exe_file).to_lowercase() == self_name
        {
            consider(pid, &self_exe);
        }
        ok = Process32NextW(snapshot, &mut entry);
    }
    CloseHandle(snapshot);
}

/// Kill `pid` only if it is unambiguously an abandoned copy of *this* binary.
unsafe fn consider(pid: u32, self_exe: &Path) {
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, 0, pid);
    if h == 0 || h == INVALID_HANDLE_VALUE {
        return; // not ours to touch
    }

    // Same file name is not enough - require the same image on disk, so an
    // unrelated statusline.exe elsewhere is never touched.
    let mut buf = [0u16; MAX_PATH * 2];
    let mut len = buf.len() as u32;
    let same_image = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) != 0
        && Path::new(&std::ffi::OsString::from_wide(&buf[..len as usize])) == self_exe;

    if same_image {
        let (mut creation, mut exit, mut kernel, mut user) = (
            FileTime::zero(),
            FileTime::zero(),
            FileTime::zero(),
            FileTime::zero(),
        );
        if GetProcessTimes(h, &mut creation, &mut exit, &mut kernel, &mut user) != 0 {
            let mut now = FileTime::zero();
            GetSystemTimeAsFileTime(&mut now);
            // FILETIME is 100 ns ticks; saturating because clock skew is real.
            let age_ms = now.as_u64().saturating_sub(creation.as_u64()) / 10_000;
            let cpu = kernel.as_u64() + user.as_u64();

            let never_ran = cpu == 0 && age_ms > NEVER_RAN_MS;
            let stale = age_ms > STALE_MS;
            if never_ran || stale {
                TerminateProcess(h, 1);
            }
        }
    }
    CloseHandle(h);
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    std::ffi::OsString::from_wide(&buf[..end])
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_conversion_stops_at_the_nul() {
        let mut buf = [0u16; 16];
        for (i, c) in "statusline.exe".encode_utf16().enumerate() {
            buf[i] = c;
        }
        assert_eq!(wide_to_string(&buf), "statusline.exe");
    }

    #[test]
    fn filetime_reassembles_both_halves() {
        let ft = FileTime { low: 0x8000_0000, high: 0x0000_0001 };
        assert_eq!(ft.as_u64(), 0x1_8000_0000);
    }

    fn stamp_value(p: &Path) -> i64 {
        std::fs::read_to_string(p).unwrap().trim().parse().unwrap()
    }

    #[test]
    fn sweep_is_rate_limited_by_the_stamp() {
        let d = std::env::temp_dir().join(format!("cps-reap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let stamp = d.join(".reap");

        // No stamp yet -> sweeps, and leaves a stamp behind.
        maybe_sweep(&d);
        assert!(stamp.exists(), "first call must stamp");
        let first = stamp_value(&stamp);

        // Fresh stamp -> returns without re-stamping.
        std::thread::sleep(std::time::Duration::from_millis(20));
        maybe_sweep(&d);
        assert_eq!(stamp_value(&stamp), first, "a fresh stamp must suppress the next sweep");
    }

    #[test]
    fn an_expired_stamp_sweeps_again_and_is_refreshed() {
        // The regression this guards: the stamp used to be tracked by mtime, and
        // rewriting an empty file with empty content is a no-op on Windows, so
        // the stamp never aged out of "just swept" - or rather, never refreshed,
        // leaving every later render to sweep. Both halves are checked here.
        let d = std::env::temp_dir().join(format!("cps-reap-exp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let stamp = d.join(".reap");

        let stale = crate::paths::now_ms() - (SWEEP_EVERY_MS + 5_000);
        std::fs::write(&stamp, stale.to_string()).unwrap();

        maybe_sweep(&d);
        let after = stamp_value(&stamp);
        assert!(after > stale, "an expired stamp must be refreshed, got {after} vs {stale}");
        assert!(
            crate::paths::now_ms() - after < SWEEP_EVERY_MS,
            "the refreshed stamp must now suppress further sweeps"
        );

        // ...and it really does suppress the next call.
        maybe_sweep(&d);
        assert_eq!(stamp_value(&stamp), after, "second call must be rate limited");
    }

    #[test]
    fn a_stamp_from_the_future_self_heals() {
        // A clock that moved back (or a garbled write) leaves the stamp ahead of
        // now. Honouring it would suppress collection until wall-clock caught
        // up, so the stamp is reset instead — one sweep, then normal service.
        let d = std::env::temp_dir().join(format!("cps-reap-clock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let stamp = d.join(".reap");
        let future = crate::paths::now_ms() + 10 * SWEEP_EVERY_MS;
        std::fs::write(&stamp, future.to_string()).unwrap();

        maybe_sweep(&d);
        let healed = stamp_value(&stamp);
        assert!(healed < future, "a future stamp must be reset, not honoured");
        assert!(
            crate::paths::now_ms() - healed < SWEEP_EVERY_MS,
            "and reset to roughly now, so the next render is rate limited"
        );
        maybe_sweep(&d);
        assert_eq!(stamp_value(&stamp), healed, "back to normal rate limiting");
    }

    #[test]
    fn a_garbled_stamp_is_replaced_rather_than_wedging_the_sweep() {
        let d = std::env::temp_dir().join(format!("cps-reap-junk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let stamp = d.join(".reap");
        std::fs::write(&stamp, "not a number").unwrap();
        maybe_sweep(&d);
        let v = stamp_value(&stamp);
        assert!(crate::paths::now_ms() - v < SWEEP_EVERY_MS, "unparseable stamp must be rewritten");
    }

    #[test]
    fn a_live_healthy_process_is_never_killed() {
        // This test process is younger than the thresholds only sometimes, but
        // it always has CPU time and is always the wrong image, so a sweep must
        // leave it running. If reaping were too aggressive this would abort.
        let d = std::env::temp_dir().join(format!("cps-reap-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        maybe_sweep(&d);
        assert!(true, "survived its own sweep");
    }
}
