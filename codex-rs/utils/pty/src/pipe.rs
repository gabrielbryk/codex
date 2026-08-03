use std::collections::HashMap;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::process::ChildTerminator;
use crate::process::ProcessHandle;
use crate::process::ProcessSignal;
use crate::process::SpawnedProcess;
use crate::process::exit_code_from_status;

#[cfg(target_os = "linux")]
use libc;

#[cfg(windows)]
enum WindowsChildTerminator {
    Job(Arc<crate::win::JobObject>),
    Process(u32),
}

struct PipeChildTerminator {
    #[cfg(windows)]
    windows: WindowsChildTerminator,
    #[cfg(unix)]
    process_group_id: u32,
}

impl ChildTerminator for PipeChildTerminator {
    fn signal(&mut self, signal: ProcessSignal) -> io::Result<()> {
        match signal {
            ProcessSignal::Interrupt => {
                #[cfg(unix)]
                {
                    crate::process_group::interrupt_process_group(self.process_group_id)
                }

                #[cfg(windows)]
                {
                    self.kill()
                }

                #[cfg(not(any(unix, windows)))]
                {
                    Err(crate::process::unsupported_signal(signal))
                }
            }
        }
    }

    fn kill(&mut self) -> io::Result<()> {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            crate::process_group::kill_process_group(self.process_group_id)
        }

        #[cfg(target_os = "macos")]
        {
            crate::process_group::kill_process_group_with_member_fallback(self.process_group_id)
        }

        #[cfg(windows)]
        {
            match &self.windows {
                WindowsChildTerminator::Job(job) => job.terminate(),
                WindowsChildTerminator::Process(pid) => kill_process(*pid),
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            Ok(())
        }
    }
}

#[cfg(windows)]
fn kill_process(pid: u32) -> io::Result<()> {
    unsafe {
        let handle = winapi::um::processthreadsapi::OpenProcess(
            winapi::um::winnt::PROCESS_TERMINATE,
            0,
            pid,
        );
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let success = winapi::um::processthreadsapi::TerminateProcess(handle, 1);
        let err = io::Error::last_os_error();
        winapi::um::handleapi::CloseHandle(handle);
        if success == 0 { Err(err) } else { Ok(()) }
    }
}

async fn read_output_stream<R>(mut reader: R, output_tx: mpsc::Sender<Vec<u8>>)
where
    R: AsyncRead + Unpin,
{
    let mut buf = vec![0u8; 8_192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let _ = output_tx.send(buf[..n].to_vec()).await;
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

#[derive(Clone, Copy)]
enum PipeSpawnMode {
    Piped,
    NullStdin,
    Contained,
}

async fn spawn_process_with_stdin_mode(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    arg0: &Option<String>,
    inherited_fds: &[i32],
    spawn_mode: PipeSpawnMode,
) -> Result<SpawnedProcess> {
    if program.is_empty() {
        anyhow::bail!("missing program for pipe spawn");
    }

    #[cfg(not(unix))]
    let _ = inherited_fds;

    let mut command = Command::new(program);
    #[cfg(unix)]
    if let Some(arg0) = arg0 {
        command.arg0(arg0);
    }
    #[cfg(target_os = "linux")]
    let parent_pid = unsafe { libc::getpid() };
    #[cfg(unix)]
    let inherited_fds = inherited_fds.to_vec();
    #[cfg(unix)]
    unsafe {
        command.pre_exec(move || {
            crate::process_group::detach_from_tty()?;
            #[cfg(target_os = "linux")]
            crate::process_group::set_parent_death_signal(parent_pid)?;
            crate::pty::close_inherited_fds_except(&inherited_fds);
            Ok(())
        });
    }
    #[cfg(not(unix))]
    let _ = arg0;
    command.current_dir(cwd);
    command.env_clear();
    for (key, value) in env {
        command.env(key, value);
    }
    for arg in args {
        command.arg(arg);
    }
    match spawn_mode {
        PipeSpawnMode::Piped | PipeSpawnMode::Contained => {
            command.stdin(Stdio::piped());
        }
        PipeSpawnMode::NullStdin => {
            command.stdin(Stdio::null());
        }
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    #[cfg(windows)]
    let job = match match spawn_mode {
        PipeSpawnMode::Contained => crate::win::JobObject::create_contained(),
        PipeSpawnMode::Piped | PipeSpawnMode::NullStdin => crate::win::JobObject::create(),
    }
    .map(Arc::new)
    {
        Ok(job) => Some(job),
        Err(err) => match spawn_mode {
            PipeSpawnMode::Contained => return Err(err.into()),
            PipeSpawnMode::Piped | PipeSpawnMode::NullStdin => {
                log::warn!("Windows pipe process tree containment unavailable: {err}");
                None
            }
        },
    };
    #[cfg(windows)]
    let suspended_spawn = matches!(spawn_mode, PipeSpawnMode::Contained);
    #[cfg(windows)]
    if job.is_some() && suspended_spawn {
        crate::win::configure_suspended_spawn(&mut command);
    }
    #[cfg(not(windows))]
    let _ = spawn_mode;

    let mut child = command.spawn()?;
    #[cfg(windows)]
    let windows_terminator = {
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("missing child pid"))?;
        if let Some(job) = job {
            let assignment_result = child
                .raw_handle()
                .ok_or_else(|| io::Error::other("missing child process handle"))
                .and_then(|process_handle| job.assign_process(process_handle));

            if let Err(err) = assignment_result.as_ref()
                && suspended_spawn
            {
                let _ = child.start_kill();
                return Err(io::Error::new(
                    err.kind(),
                    format!("failed to contain suspended process {pid}: {err}"),
                )
                .into());
            }

            if suspended_spawn && let Err(err) = crate::win::resume_suspended_process(pid) {
                let _ = job.terminate();
                let _ = child.start_kill();
                return Err(io::Error::new(
                    err.kind(),
                    format!("failed to resume contained process {pid}: {err}"),
                )
                .into());
            }

            match assignment_result {
                Ok(()) => WindowsChildTerminator::Job(job),
                Err(err) => {
                    log::warn!(
                        "Windows pipe process tree containment unavailable for pid {pid}: {err}"
                    );
                    WindowsChildTerminator::Process(pid)
                }
            }
        } else {
            WindowsChildTerminator::Process(pid)
        }
    };
    #[cfg(unix)]
    let process_group_id = child
        .id()
        .ok_or_else(|| io::Error::other("missing child pid"))?;

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (writer_tx, mut writer_rx) = mpsc::channel::<Vec<u8>>(128);
    let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>(128);
    let (stderr_tx, stderr_rx) = mpsc::channel::<Vec<u8>>(128);
    let writer_handle = if let Some(stdin) = stdin {
        tokio::spawn(async move {
            let mut writer = stdin;
            while let Some(bytes) = writer_rx.recv().await {
                let _ = writer.write_all(&bytes).await;
                let _ = writer.flush().await;
            }
        })
    } else {
        drop(writer_rx);
        tokio::spawn(async {})
    };

    let stdout_handle = stdout.map(|stdout| {
        let stdout_tx = stdout_tx.clone();
        tokio::spawn(async move {
            read_output_stream(BufReader::new(stdout), stdout_tx).await;
        })
    });
    let stderr_handle = stderr.map(|stderr| {
        let stderr_tx = stderr_tx.clone();
        tokio::spawn(async move {
            read_output_stream(BufReader::new(stderr), stderr_tx).await;
        })
    });
    let mut reader_abort_handles = Vec::new();
    if let Some(handle) = stdout_handle.as_ref() {
        reader_abort_handles.push(handle.abort_handle());
    }
    if let Some(handle) = stderr_handle.as_ref() {
        reader_abort_handles.push(handle.abort_handle());
    }
    let reader_handle = tokio::spawn(async move {
        if let Some(handle) = stdout_handle {
            let _ = handle.await;
        }
        if let Some(handle) = stderr_handle {
            let _ = handle.await;
        }
    });

    let (exit_tx, exit_rx) = oneshot::channel::<i32>();
    let exit_status = Arc::new(AtomicBool::new(false));
    let wait_exit_status = Arc::clone(&exit_status);
    let exit_code = Arc::new(StdMutex::new(None));
    let wait_exit_code = Arc::clone(&exit_code);
    #[cfg(windows)]
    let wait_job = match (&windows_terminator, spawn_mode) {
        (WindowsChildTerminator::Job(job), PipeSpawnMode::Piped | PipeSpawnMode::NullStdin) => {
            Some(Arc::clone(job))
        }
        (WindowsChildTerminator::Job(_), PipeSpawnMode::Contained)
        | (WindowsChildTerminator::Process(_), _) => None,
    };
    let wait_handle: JoinHandle<()> = tokio::spawn(async move {
        let code = match child.wait().await {
            Ok(status) => {
                #[cfg(windows)]
                if let Some(job) = wait_job
                    && let Err(err) = job.preserve_descendants()
                {
                    log::warn!(
                        "Windows pipe failed to preserve descendants after root exit: {err}"
                    );
                }
                exit_code_from_status(status)
            }
            Err(_) => -1,
        };
        wait_exit_status.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut guard) = wait_exit_code.lock() {
            *guard = Some(code);
        }
        let _ = exit_tx.send(code);
    });

    let handle = ProcessHandle::new(
        writer_tx,
        Box::new(PipeChildTerminator {
            #[cfg(windows)]
            windows: windows_terminator,
            #[cfg(unix)]
            process_group_id,
        }),
        reader_handle,
        reader_abort_handles,
        writer_handle,
        wait_handle,
        exit_status,
        exit_code,
        /*pty_handles*/ None,
        /*resizer*/ None,
    );

    Ok(SpawnedProcess {
        session: handle,
        stdout_rx,
        stderr_rx,
        exit_rx,
    })
}

/// Spawn a process using regular pipes and preserve selected inherited file
/// descriptors across exec on Unix.
pub async fn spawn_process(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    arg0: &Option<String>,
    inherited_fds: &[i32],
) -> Result<SpawnedProcess> {
    spawn_process_with_stdin_mode(
        program,
        args,
        cwd,
        env,
        arg0,
        inherited_fds,
        PipeSpawnMode::Piped,
    )
    .await
}

/// Spawn a process using regular pipes, close stdin immediately, and preserve
/// selected inherited file descriptors across exec on Unix.
pub async fn spawn_process_no_stdin(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    arg0: &Option<String>,
    inherited_fds: &[i32],
) -> Result<SpawnedProcess> {
    spawn_process_with_stdin_mode(
        program,
        args,
        cwd,
        env,
        arg0,
        inherited_fds,
        PipeSpawnMode::NullStdin,
    )
    .await
}

/// Spawn a non-interactive process contained as tightly as the platform allows,
/// terminable through the returned [`ProcessHandle`].
///
/// Containment strength is platform-specific and is *not* uniform:
///
/// - **Windows**: the child is created suspended, assigned to a Job Object that
///   forbids breakaway, and only then resumed, so no descendant can escape
///   during spawn. Termination kills the whole tree. Unlike [`spawn_process`],
///   this function fails rather than falling back to root-process-only
///   termination when Job Object containment cannot be established. Descendants
///   remain owned by the returned session after the root exits and are
///   terminated when that session is terminated or dropped.
/// - **Unix**: the child leads its own process group and termination signals
///   that group. This is **best-effort**: a descendant that calls `setsid()` (or
///   otherwise leaves the group) escapes cleanup. Daemonizing children are not
///   supported by this API.
pub async fn spawn_contained_process(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    arg0: &Option<String>,
    inherited_fds: &[i32],
) -> Result<SpawnedProcess> {
    spawn_process_with_stdin_mode(
        program,
        args,
        cwd,
        env,
        arg0,
        inherited_fds,
        PipeSpawnMode::Contained,
    )
    .await
}

#[cfg(all(test, windows))]
#[path = "pipe_tests.rs"]
mod tests;
