use crate::capture::ScreenCapturer;
use crate::input::InputInjector;
use crate::protocol::RemoteInput;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, ERROR_SERVICE_ALREADY_RUNNING,
    ERROR_SERVICE_DOES_NOT_EXIST, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, STILL_ACTIVE,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, SetTokenInformation, TokenPrimary, TokenSessionId,
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ALL_ACCESS,
};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_WAIT,
};
use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;
use windows::Win32::System::Services::{
    ChangeServiceConfigW, CreateServiceW, OpenSCManagerW, OpenServiceW, RegisterServiceCtrlHandlerW,
    SetServiceStatus, StartServiceCtrlDispatcherW, StartServiceW, ENUM_SERVICE_TYPE,
    SC_MANAGER_ALL_ACCESS, SERVICE_ALL_ACCESS, SERVICE_CONTROL_INTERROGATE,
    SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_DEMAND_START, SERVICE_ERROR,
    SERVICE_ERROR_NORMAL, SERVICE_NO_CHANGE, SERVICE_RUNNING, SERVICE_START_TYPE, SERVICE_STATUS,
    SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING, SERVICE_TABLE_ENTRYW,
    SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken,
    TerminateProcess, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTUPINFOW,
};

const SERVICE_NAME: &str = "LusidDesktopService";
const PIPE_NAME: &str = r"\\.\pipe\LusidDesktopAgent";

static SERVICE_STOPPING: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: OnceLock<usize> = OnceLock::new();

#[derive(Serialize, Deserialize)]
enum AgentRequest {
    CaptureFrame,
    Input(RemoteInput),
}

#[derive(Serialize, Deserialize)]
enum AgentResponse {
    Frame {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Ack,
    Error(String),
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn write_framed<W: Write, T: Serialize>(writer: &mut W, msg: &T) -> std::io::Result<()> {
    let bytes = bincode::serialize(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len = (bytes.len() as u32).to_le_bytes();
    writer.write_all(&len)?;
    writer.write_all(&bytes)?;
    writer.flush()
}

fn read_framed<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> std::io::Result<T> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "IPC frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    bincode::deserialize(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Host tarafının SYSTEM masaüstü ajanına bağlanmak için kullandığı istemci.
pub struct DesktopAgentClient {
    pipe: File,
}

impl DesktopAgentClient {
    pub fn connect() -> Option<Self> {
        let pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(PIPE_NAME)
            .ok()?;
        Some(Self { pipe })
    }

    pub fn capture_frame(&mut self) -> Result<(u32, u32, Vec<u8>), String> {
        write_framed(&mut self.pipe, &AgentRequest::CaptureFrame)
            .map_err(|e| format!("IPC yazma hatası: {}", e))?;
        let resp: AgentResponse =
            read_framed(&mut self.pipe).map_err(|e| format!("IPC okuma hatası: {}", e))?;
        match resp {
            AgentResponse::Frame {
                width,
                height,
                rgba,
            } => Ok((width, height, rgba)),
            AgentResponse::Error(e) => Err(e),
            AgentResponse::Ack => Err("Beklenmeyen yanıt".into()),
        }
    }

    pub fn send_input(&mut self, input: RemoteInput) -> Result<(), String> {
        write_framed(&mut self.pipe, &AgentRequest::Input(input))
            .map_err(|e| format!("IPC yazma hatası: {}", e))?;
        let resp: AgentResponse =
            read_framed(&mut self.pipe).map_err(|e| format!("IPC okuma hatası: {}", e))?;
        match resp {
            AgentResponse::Ack => Ok(()),
            AgentResponse::Error(e) => Err(e),
            _ => Ok(()),
        }
    }
}

pub fn is_desktop_agent_available() -> bool {
    DesktopAgentClient::connect().is_some()
}

/// Yönetici yetkisiyle çalışıyorsa `LusidDesktopService` Windows hizmetini kurar/günceller ve başlatır.
/// Standart kullanıcı yetkisinde sessizce false döner.
pub fn ensure_system_service_running() -> bool {
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let bin_cmd = format!("\"{}\" --service", exe_path.display());
    let bin_wide = to_wide(&bin_cmd);
    let svc_name_wide = to_wide(SERVICE_NAME);
    let display_name_wide = to_wide("Lusid Remote Desktop UAC Service");

    unsafe {
        let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) else {
            return false;
        };

        let svc_res = OpenServiceW(scm, PCWSTR(svc_name_wide.as_ptr()), SERVICE_ALL_ACCESS);
        let svc = match svc_res {
            Ok(existing) => {
                // Exe başka bir klasöre taşındıysa yolunu güncelle
                let _ = ChangeServiceConfigW(
                    existing,
                    ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                    SERVICE_START_TYPE(SERVICE_NO_CHANGE),
                    SERVICE_ERROR(SERVICE_NO_CHANGE),
                    PCWSTR(bin_wide.as_ptr()),
                    PCWSTR::null(),
                    None,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    PCWSTR::null(),
                );
                existing
            }
            Err(_) => {
                let err = GetLastError();
                if err != ERROR_SERVICE_DOES_NOT_EXIST {
                    let _ = windows::Win32::System::Services::CloseServiceHandle(scm);
                    return false;
                }
                match CreateServiceW(
                    scm,
                    PCWSTR(svc_name_wide.as_ptr()),
                    PCWSTR(display_name_wide.as_ptr()),
                    SERVICE_ALL_ACCESS,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_DEMAND_START,
                    SERVICE_ERROR_NORMAL,
                    PCWSTR(bin_wide.as_ptr()),
                    PCWSTR::null(),
                    None,
                    PCWSTR::null(),
                    PCWSTR::null(), // LocalSystem
                    PCWSTR::null(),
                ) {
                    Ok(created) => created,
                    Err(_) => {
                        let _ = windows::Win32::System::Services::CloseServiceHandle(scm);
                        return false;
                    }
                }
            }
        };

        let started = match StartServiceW(svc, None) {
            Ok(_) => true,
            Err(_) => GetLastError() == ERROR_SERVICE_ALREADY_RUNNING,
        };

        let _ = windows::Win32::System::Services::CloseServiceHandle(svc);
        let _ = windows::Win32::System::Services::CloseServiceHandle(scm);
        started
    }
}

/// `--desktop-agent`: Aktif kullanıcı oturumunda `NT AUTHORITY\SYSTEM` olarak çalışır.
/// Penceresizdir; bu sayede `OpenInputDesktop` + `SetThreadDesktop` ile Winlogon (UAC) masaüstüne geçebilir.
pub fn run_desktop_agent() {
    let pipe_name_wide = to_wide(PIPE_NAME);
    // SYSTEM (SY) ve Administrators (BA) tam erişim
    let sddl_wide = to_wide("D:(A;;GA;;;SY)(A;;GA;;;BA)");

    loop {
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            let sd_ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl_wide.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .is_ok();

            let mut sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: if sd_ok { sd.0 } else { std::ptr::null_mut() },
                bInheritHandle: false.into(),
            };

            let pipe_handle = CreateNamedPipeW(
                PCWSTR(pipe_name_wide.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                8,
                16 * 1024 * 1024,
                64 * 1024,
                0,
                if sd_ok { Some(&mut sa) } else { None },
            );

            if sd_ok && !sd.0.is_null() {
                let _ = LocalFree(HLOCAL(sd.0));
            }

            if pipe_handle == INVALID_HANDLE_VALUE {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }

            let connected = match ConnectNamedPipe(pipe_handle, None) {
                Ok(_) => true,
                Err(_) => GetLastError() == ERROR_PIPE_CONNECTED,
            };

            if !connected {
                let _ = CloseHandle(pipe_handle);
                continue;
            }

            // Her istemci bağlantısını kendi penceresiz thread'inde işle
            let raw_handle = pipe_handle.0 as usize;
            std::thread::spawn(move || {
                let handle = HANDLE(raw_handle as *mut _);
                let mut file = File::from_raw_handle(raw_handle as *mut _);
                let mut capturer = ScreenCapturer::new().unwrap_or_default();

                loop {
                    let req: AgentRequest = match read_framed(&mut file) {
                        Ok(r) => r,
                        Err(_) => break,
                    };

                    let resp = match req {
                        AgentRequest::CaptureFrame => match capturer.try_capture_live_frame() {
                            Ok(frame) => AgentResponse::Frame {
                                width: frame.width,
                                height: frame.height,
                                rgba: frame.rgba,
                            },
                            Err(e) => AgentResponse::Error(e),
                        },
                        AgentRequest::Input(ev) => {
                            InputInjector::execute(ev);
                            AgentResponse::Ack
                        }
                    };

                    if write_framed(&mut file, &resp).is_err() {
                        break;
                    }
                }

                let _ = FlushFileBuffers(handle);
                let _ = DisconnectNamedPipe(handle);
                // `file` drop edilirken handle'ı kapatır.
                std::mem::forget(handle);
            });
        }
    }
}

/// `--service`: Session 0'da `LocalSystem` olarak çalışan Windows Hizmeti.
pub fn run_windows_service() {
    let mut name_wide = to_wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name_wide.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR::null(),
            lpServiceProc: None,
        },
    ];

    unsafe {
        let _ = StartServiceCtrlDispatcherW(table.as_ptr());
    }
}

fn set_service_state(state: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE) {
    if let Some(&raw) = STATUS_HANDLE.get() {
        let status_handle = SERVICE_STATUS_HANDLE(raw as *mut _);
        let status = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: if state == SERVICE_RUNNING { 1 | 4 } else { 0 }, // STOP | SHUTDOWN
            dwWin32ExitCode: 0,
            dwServiceSpecificExitCode: 0,
            dwCheckPoint: 0,
            dwWaitHint: 0,
        };
        unsafe {
            let _ = SetServiceStatus(status_handle, &status);
        }
    }
}

unsafe extern "system" fn service_ctrl_handler(control: u32) {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            SERVICE_STOPPING.store(true, Ordering::SeqCst);
            set_service_state(SERVICE_STOP_PENDING);
        }
        SERVICE_CONTROL_INTERROGATE => {}
        _ => {}
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let svc_name = to_wide(SERVICE_NAME);
    if let Ok(handle) = RegisterServiceCtrlHandlerW(PCWSTR(svc_name.as_ptr()), Some(service_ctrl_handler)) {
        let _ = STATUS_HANDLE.set(handle.0 as usize);
    }
    set_service_state(SERVICE_RUNNING);

    let exe_path = std::env::current_exe().ok();
    let mut child_process: Option<(u32, HANDLE)> = None;

    while !SERVICE_STOPPING.load(Ordering::SeqCst) {
        let active_session = WTSGetActiveConsoleSessionId();

        // Eski çocuk süreç hala ayakta mı ve aynı oturumda mı kontrol et
        let need_spawn = if active_session == 0xFFFFFFFF {
            false
        } else if let Some((sess, hproc)) = child_process {
            let mut exit_code = 0u32;
            let alive = GetExitCodeProcess(hproc, &mut exit_code).is_ok()
                && exit_code == STILL_ACTIVE.0 as u32;
            if !alive || sess != active_session {
                if alive {
                    let _ = TerminateProcess(hproc, 0);
                }
                let _ = CloseHandle(hproc);
                child_process = None;
                true
            } else {
                false
            }
        } else {
            true
        };

        if need_spawn {
            if let Some(ref exe) = exe_path {
                if let Some(hproc) = spawn_system_agent_in_session(exe, active_session) {
                    child_process = Some((active_session, hproc));
                }
            }
        }

        std::thread::sleep(Duration::from_millis(1000));
    }

    if let Some((_, hproc)) = child_process {
        let _ = TerminateProcess(hproc, 0);
        let _ = CloseHandle(hproc);
    }
    set_service_state(SERVICE_STOPPED);
}

unsafe fn spawn_system_agent_in_session(exe: &std::path::Path, session_id: u32) -> Option<HANDLE> {
    let mut current_token = HANDLE::default();
    OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut current_token).ok()?;

    let mut dup_token = HANDLE::default();
    let dup_ok = DuplicateTokenEx(
        current_token,
        TOKEN_ALL_ACCESS,
        None,
        SecurityImpersonation,
        TokenPrimary,
        &mut dup_token,
    )
    .is_ok();
    let _ = CloseHandle(current_token);
    if !dup_ok {
        return None;
    }

    let sess_ok = SetTokenInformation(
        dup_token,
        TokenSessionId,
        &session_id as *const u32 as *const _,
        std::mem::size_of::<u32>() as u32,
    )
    .is_ok();

    if !sess_ok {
        let _ = CloseHandle(dup_token);
        return None;
    }

    let mut desktop_wide = to_wide(r"winsta0\default");
    let mut cmd_wide = to_wide(&format!("\"{}\" --desktop-agent", exe.display()));

    let mut si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: PWSTR(desktop_wide.as_mut_ptr()),
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();

    let created = CreateProcessAsUserW(
        dup_token,
        PCWSTR::null(),
        PWSTR(cmd_wide.as_mut_ptr()),
        None,
        None,
        false,
        CREATE_NO_WINDOW,
        None,
        PCWSTR::null(),
        &mut si,
        &mut pi,
    )
    .is_ok();

    let _ = CloseHandle(dup_token);

    if created {
        let _ = CloseHandle(pi.hThread);
        Some(pi.hProcess)
    } else {
        None
    }
}
