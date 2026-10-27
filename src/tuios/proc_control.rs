/// Signals for processes tuiOS did not spawn through a PTY handle (apps in their own
/// terminal window), plus suspend / resume for any PID.

/// Ask a process to exit. Windows has no equivalent for a console app in another
/// console, so callers fall straight through to `kill` there.
pub fn terminate(pid: u32) -> bool {
    imp::terminate(pid)
}

pub fn kill(pid: u32) -> bool {
    imp::kill(pid)
}

/// Freeze a process. `group` also freezes the processes it started (Unix only — a PTY
/// child leads its own process group).
pub fn suspend(pid: u32, group: bool) -> bool {
    imp::suspend(pid, group)
}

pub fn resume(pid: u32, group: bool) -> bool {
    imp::resume(pid, group)
}

pub fn is_alive(pid: u32) -> bool {
    imp::is_alive(pid)
}

/// Whether `terminate` can do anything gentler than `kill` on this platform.
pub fn has_graceful_terminate() -> bool {
    cfg!(unix)
}

#[cfg(unix)]
mod imp {
    fn signal(pid: u32, sig: libc::c_int, group: bool) -> bool {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }
        let target = if group { -(pid as i32) } else { pid as i32 };
        // SAFETY: kill(2) only takes integers; an invalid pid just returns an error
        unsafe { libc::kill(target, sig) == 0 }
    }

    pub fn terminate(pid: u32) -> bool {
        signal(pid, libc::SIGTERM, false)
    }

    pub fn kill(pid: u32) -> bool {
        signal(pid, libc::SIGKILL, false)
    }

    pub fn suspend(pid: u32, group: bool) -> bool {
        signal(pid, libc::SIGSTOP, group) || (group && signal(pid, libc::SIGSTOP, false))
    }

    pub fn resume(pid: u32, group: bool) -> bool {
        signal(pid, libc::SIGCONT, group) || (group && signal(pid, libc::SIGCONT, false))
    }

    pub fn is_alive(pid: u32) -> bool {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }
        // Signal 0 checks existence; EPERM still means the process is there
        unsafe {
            libc::kill(pid as i32, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_SUSPEND_RESUME: u32 = 0x0800;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn TerminateProcess(process: *mut c_void, exit_code: u32) -> i32;
        fn GetExitCodeProcess(process: *mut c_void, exit_code: *mut u32) -> i32;
    }

    // Undocumented but long-stable; what Resource Monitor's "Suspend process" uses
    #[link(name = "ntdll")]
    extern "system" {
        fn NtSuspendProcess(process: *mut c_void) -> i32;
        fn NtResumeProcess(process: *mut c_void) -> i32;
    }

    fn with_handle<T>(pid: u32, access: u32, f: impl FnOnce(*mut c_void) -> T) -> Option<T> {
        if pid == 0 {
            return None;
        }
        // SAFETY: the handle is checked for null and closed after use
        unsafe {
            let handle = OpenProcess(access, 0, pid);
            if handle.is_null() {
                return None;
            }
            let out = f(handle);
            CloseHandle(handle);
            Some(out)
        }
    }

    pub fn terminate(_pid: u32) -> bool {
        false
    }

    pub fn kill(pid: u32) -> bool {
        with_handle(pid, PROCESS_TERMINATE, |h| unsafe { TerminateProcess(h, 1) != 0 }).unwrap_or(false)
    }

    pub fn suspend(pid: u32, _group: bool) -> bool {
        with_handle(pid, PROCESS_SUSPEND_RESUME, |h| unsafe { NtSuspendProcess(h) >= 0 }).unwrap_or(false)
    }

    pub fn resume(pid: u32, _group: bool) -> bool {
        with_handle(pid, PROCESS_SUSPEND_RESUME, |h| unsafe { NtResumeProcess(h) >= 0 }).unwrap_or(false)
    }

    pub fn is_alive(pid: u32) -> bool {
        with_handle(pid, PROCESS_QUERY_LIMITED_INFORMATION, |h| {
            let mut code = 0u32;
            unsafe { GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE }
        })
        .unwrap_or(false)
    }
}
