//! "Start with Windows": a value under
//! `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! This is the per-user startup list (no admin rights needed). Windows runs it after
//! the user signs in; the player then opens straight into fullscreen, looping the last
//! playlist (`--autostart`).

/// Only implemented on Windows.
pub const AVAILABLE: bool = cfg!(windows);

/// The command line registered for start-up.
#[cfg_attr(not(windows), allow(dead_code))]
fn command_line() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --autostart", exe.display()))
}

#[cfg(windows)]
mod imp {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::*;

    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const VALUE: PCWSTR = w!("A-Player");

    /// The registered command line, if any.
    pub fn registered() -> Option<String> {
        unsafe {
            let mut size = 0u32;
            if RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            ) != ERROR_SUCCESS
            {
                return None;
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2)];
            if RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            ) != ERROR_SUCCESS
            {
                return None;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..len]))
        }
    }

    pub fn register(command: &str) -> Result<(), String> {
        let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        let err = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                (wide.len() * 2) as u32,
            )
        };
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "Could not add to Windows start-up (error {})",
                err.0
            ))
        }
    }

    pub fn unregister() -> Result<(), String> {
        let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
        if err == ERROR_SUCCESS || err.0 == 2 {
            Ok(()) // 2 = already absent
        } else {
            Err(format!(
                "Could not remove from Windows start-up (error {})",
                err.0
            ))
        }
    }
}

pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        imp::registered().is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        if on {
            imp::register(&command_line().ok_or("Cannot find the program's own path")?)
        } else {
            imp::unregister()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err("Start with Windows is only available on Windows".into())
    }
}

/// If start-up is enabled but the program was moved, point the entry at the new location.
pub fn refresh() {
    #[cfg(windows)]
    if let (Some(current), Some(wanted)) = (imp::registered(), command_line()) {
        if current != wanted {
            if let Err(e) = imp::register(&wanted) {
                log::warn!("{e}");
            }
        }
    }
}
