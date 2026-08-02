use std::io;
use std::mem;

use filedescriptor::OwnedHandle;
use std::os::windows::io::FromRawHandle;
use tokio::process::Command;
use winapi::shared::minwindef::FALSE;
use winapi::um::processthreadsapi::OpenThread;
use winapi::um::processthreadsapi::ResumeThread;
use winapi::um::tlhelp32::CreateToolhelp32Snapshot;
use winapi::um::tlhelp32::TH32CS_SNAPTHREAD;
use winapi::um::tlhelp32::THREADENTRY32;
use winapi::um::tlhelp32::Thread32First;
use winapi::um::tlhelp32::Thread32Next;
use winapi::um::winbase::CREATE_SUSPENDED;
use winapi::um::winnt::HANDLE;
use winapi::um::winnt::THREAD_SUSPEND_RESUME;

/// Configure a command so no user code runs before its process is contained.
pub(crate) fn configure_suspended_spawn(command: &mut Command) {
    command.creation_flags(CREATE_SUSPENDED);
}

/// Resume the initial thread of a process created with `CREATE_SUSPENDED`.
///
/// Tokio does not expose the primary thread handle returned by `CreateProcessW`,
/// so locate the sole initial thread while the new process is still suspended.
pub(crate) fn resume_suspended_process(process_id: u32) -> io::Result<()> {
    let snapshot = unsafe {
        CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, /*th32ProcessID*/ 0)
    };
    if snapshot == winapi::um::handleapi::INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.cast()) };

    let mut entry: THREADENTRY32 = unsafe { mem::zeroed() };
    entry.dwSize = mem::size_of::<THREADENTRY32>() as u32;
    if unsafe { Thread32First(snapshot_handle(&snapshot), &mut entry) } == FALSE {
        return Err(io::Error::last_os_error());
    }

    loop {
        if entry.th32OwnerProcessID == process_id {
            return resume_thread(entry.th32ThreadID);
        }
        if unsafe { Thread32Next(snapshot_handle(&snapshot), &mut entry) } == FALSE {
            break;
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("could not find initial thread for suspended process {process_id}"),
    ))
}

fn snapshot_handle(snapshot: &OwnedHandle) -> HANDLE {
    use std::os::windows::io::AsRawHandle;

    snapshot.as_raw_handle().cast()
}

fn resume_thread(thread_id: u32) -> io::Result<()> {
    let thread = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME,
            /*bInheritHandle*/ FALSE,
            thread_id,
        )
    };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    let thread = unsafe { OwnedHandle::from_raw_handle(thread.cast()) };
    use std::os::windows::io::AsRawHandle;
    let previous_suspend_count = unsafe { ResumeThread(thread.as_raw_handle().cast()) };
    if previous_suspend_count == u32::MAX {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
