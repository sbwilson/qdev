//! Cross-platform process group creation (Unix) and job object attachment (Windows).
//! Guarantees clean child/grandchild termination on timeout.

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject,
    JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

#[cfg(unix)]
pub struct ProcessGroupIsolation;

#[cfg(unix)]
impl ProcessGroupIsolation {
    /// Puts the spawned child process into a new process group via `.process_group(0)`.
    pub fn configure_command(cmd: &mut std::process::Command) {
        cmd.process_group(0);
    }

    pub fn on_spawned(_child: &std::process::Child) -> Self {
        Self
    }

    /// Kills the child process and all descendants by sending SIGKILL to the process group.
    pub fn kill_tree(&self, child: &mut std::process::Child) {
        let pid = child.id() as i32;
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = child.kill();
    }
}

#[cfg(windows)]
pub struct ProcessGroupIsolation {
    job_handle: HANDLE,
}

#[cfg(windows)]
impl ProcessGroupIsolation {
    pub fn configure_command(_cmd: &mut std::process::Command) {}

    pub fn on_spawned(child: &std::process::Child) -> Self {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job != std::ptr::null_mut() && job != INVALID_HANDLE_VALUE as _ {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let res = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if res != 0 {
                    let raw_handle = child.as_raw_handle() as HANDLE;
                    let _ = AssignProcessToJobObject(job, raw_handle);
                }
            }
            Self { job_handle: job }
        }
    }

    pub fn kill_tree(&self, child: &mut std::process::Child) {
        if self.job_handle != std::ptr::null_mut() && self.job_handle != INVALID_HANDLE_VALUE as _ {
            unsafe {
                TerminateJobObject(self.job_handle, 1);
            }
        }
        let _ = child.kill();
    }
}

#[cfg(windows)]
impl Drop for ProcessGroupIsolation {
    fn drop(&mut self) {
        if self.job_handle != std::ptr::null_mut() && self.job_handle != INVALID_HANDLE_VALUE as _ {
            unsafe {
                CloseHandle(self.job_handle);
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
pub struct ProcessGroupIsolation;

#[cfg(not(any(unix, windows)))]
impl ProcessGroupIsolation {
    pub fn configure_command(_cmd: &mut std::process::Command) {}

    pub fn on_spawned(_child: &std::process::Child) -> Self {
        Self
    }

    pub fn kill_tree(&self, child: &mut std::process::Child) {
        let _ = child.kill();
    }
}
