//! Custody of one foreground command. No process-name searches or global signals.
//! Windows assigns the suspended child to a private kill-on-close job before it can run.
//! Unix uses a private process group; deliberately detached sessions are outside that contract.

use std::io;

#[cfg(windows)]
mod platform {
    use super::*;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::*;

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub struct Tree {
        job: Handle,
        child: Child,
    }

    // The child has never run: its primary thread cannot exit or its PID be recycled.
    // Use the standard library to create it, preserving Windows executable resolution,
    // batch-file escaping, Unicode arguments and standard-handle inheritance.
    fn resume(child: &Child) -> io::Result<()> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let snapshot = Handle(snapshot);
            let mut entry: THREADENTRY32 = zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut more = Thread32First(snapshot.0, &mut entry);
            while more != 0 {
                if entry.th32OwnerProcessID == child.id() {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if thread.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                    let thread = Handle(thread);
                    if ResumeThread(thread.0) == u32::MAX {
                        return Err(io::Error::last_os_error());
                    }
                    return Ok(());
                }
                more = Thread32Next(snapshot.0, &mut entry);
            }
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "suspended command thread not found",
            ))
        }
    }

    impl Tree {
        pub fn spawn(argv: &[String]) -> io::Result<Self> {
            unsafe {
                let job = CreateJobObjectW(null(), null());
                if job.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = Handle(job);
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let mut child = Command::new(&argv[0])
                    .args(&argv[1..])
                    .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
                    .spawn()?;
                if AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) == 0 {
                    let error = io::Error::last_os_error();
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error); // Never fall back to an uncontained command.
                }
                if let Err(error) = resume(&child) {
                    TerminateJobObject(job.0, 1);
                    let _ = child.wait();
                    return Err(error);
                }
                Ok(Self { job, child })
            }
        }
        pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
            Ok(self.child.try_wait()?.map(|s| s.code().unwrap_or(1)))
        }
        pub fn stop(&mut self) -> io::Result<()> {
            unsafe {
                if TerminateJobObject(self.job.0, 1) == 0 {
                    return Err(io::Error::last_os_error());
                }
                loop {
                    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
                    if QueryInformationJobObject(
                        self.job.0,
                        JobObjectBasicAccountingInformation,
                        &mut accounting as *mut _ as _,
                        size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                        null_mut(),
                    ) == 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    if accounting.ActiveProcesses == 0 {
                        self.child.wait()?;
                        return Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    // Darwin killpg excludes zombies and returns EPERM for a zombie-only group.
    // EPERM can also mean a live, unsignalable member: never ignore it on its own.
    #[cfg(target_os = "macos")]
    fn zombie_group(group: i32) -> io::Result<bool> {
        use std::mem::{size_of, zeroed};
        const PROC_PGRP_ONLY: u32 = 2;
        fn members(group: i32) -> io::Result<Vec<i32>> {
            unsafe {
                let bytes =
                    libc::proc_listpids(PROC_PGRP_ONLY, group as u32, std::ptr::null_mut(), 0);
                if bytes <= 0 || bytes as usize > 16 * 1024 * 1024 {
                    return Err(io::Error::other("cannot size command process group"));
                }
                let mut pids = vec![0i32; bytes as usize / size_of::<i32>() + 64];
                let capacity = (pids.len() * size_of::<i32>()) as i32;
                let used = libc::proc_listpids(
                    PROC_PGRP_ONLY,
                    group as u32,
                    pids.as_mut_ptr().cast(),
                    capacity,
                );
                // Zero also represents an API error. A pinned leader must be present.
                // A full buffer may be truncated; retry cleanup instead of guessing.
                if used <= 0
                    || used >= capacity
                    || !(used as usize).is_multiple_of(size_of::<i32>())
                {
                    return Err(io::Error::other(
                        "incomplete command process group snapshot",
                    ));
                }
                pids.truncate(used as usize / size_of::<i32>());
                Ok(pids)
            }
        }
        let snapshot = members(group)?;
        if !snapshot.contains(&group) {
            return Ok(false);
        }
        for &pid in &snapshot {
            if pid <= 0 {
                return Ok(false);
            }
            let mut info: libc::proc_bsdinfo = unsafe { zeroed() };
            // arg=1 includes zombies; arg=0 would report the pinned leader as ESRCH.
            let used = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    1,
                    &mut info as *mut _ as _,
                    size_of::<libc::proc_bsdinfo>() as i32,
                )
            };
            if used != size_of::<libc::proc_bsdinfo>() as i32
                || info.pbi_pid != pid as u32
                || info.pbi_pgid != group as u32
                || info.pbi_status != libc::SZOMB
            {
                return Ok(false);
            }
        }
        // A member could fork between the first snapshot and its transition to zombie.
        // Re-snapshot after every observed member is dead: any new PID refuses cleanup.
        // WNOWAIT still pins the leader, so this group cannot be recycled meanwhile.
        Ok(members(group)?.iter().all(|pid| snapshot.contains(pid)))
    }

    pub struct Tree {
        child: Child,
        group: i32,
        stopped: bool,
    }
    impl Tree {
        pub fn spawn(argv: &[String]) -> io::Result<Self> {
            let mut command = Command::new(&argv[0]);
            command.args(&argv[1..]);
            // SAFETY: only the async-signal-safe setpgid runs between fork and exec.
            unsafe {
                command.pre_exec(|| {
                    if libc::setpgid(0, 0) == -1 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
            let child = command.spawn()?;
            let group = child.id() as i32;
            Ok(Self {
                child,
                group,
                stopped: false,
            })
        }
        pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
            // WNOWAIT pins the group leader's PID until cleanup. Reaping it here would
            // allow a recycled PID to name an unrelated group before stop() signals it.
            unsafe {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                if libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                ) == -1
                {
                    return Err(io::Error::last_os_error());
                }
                if info.si_pid() == 0 {
                    return Ok(None);
                }
                let status = info.si_status();
                Ok(Some(if info.si_code == libc::CLD_EXITED {
                    status
                } else {
                    128 + status
                }))
            }
        }
        pub fn stop(&mut self) -> io::Result<()> {
            if self.stopped {
                return Ok(());
            }
            // The group belongs only to this command, never to the invoking agent or shell.
            if unsafe { libc::kill(-self.group, libc::SIGKILL) } == -1 {
                let e = io::Error::last_os_error();
                let gone = e.raw_os_error() == Some(libc::ESRCH);
                #[cfg(target_os = "macos")]
                let gone =
                    gone || (e.raw_os_error() == Some(libc::EPERM) && zombie_group(self.group)?);
                if !gone {
                    return Err(e);
                }
            }
            self.child.wait()?;
            self.stopped = true;
            Ok(())
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    #[cfg(all(test, target_os = "macos"))]
    mod tests {
        use super::*;
        use std::time::{Duration, Instant};

        #[test]
        fn zombie_proof_refuses_a_live_member_even_after_leader_exit() {
            // Both children are bounded; this test never inspects or signals another group.
            let mut tree =
                Tree::spawn(&["/bin/sh".into(), "-c".into(), "sleep 2 & exit 0".into()]).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while tree.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                !zombie_group(tree.group).unwrap(),
                "live child must retain custody"
            );
            tree.stop().unwrap();
            tree.stop().unwrap(); // No signal after the leader was reaped.
        }
    }
}

pub use platform::Tree;
