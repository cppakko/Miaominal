#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAuthVerification {
    Verified,
    Canceled,
    Unavailable,
    Busy,
    RetriesExhausted,
    Failed,
}

#[cfg(windows)]
pub fn is_app_foreground() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    let foreground = unsafe { GetForegroundWindow() };
    if foreground.0.is_null() {
        return false;
    }
    let mut process_id = 0_u32;
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut process_id)) };
    process_id == std::process::id()
}

#[cfg(not(windows))]
pub fn is_app_foreground() -> bool {
    true
}

#[cfg(windows)]
pub fn configure_notification_window(window: &gpui::Window) -> anyhow::Result<()> {
    use raw_window_handle::RawWindowHandle;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, HGDIOBJ, SetWindowRgn};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    let handle = raw_window_handle::HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("failed to access notification window handle: {error:?}"))?
        .as_raw();
    let RawWindowHandle::Win32(handle) = handle else {
        anyhow::bail!("GPUI did not expose a Win32 notification window handle");
    };
    let hwnd = HWND(handle.hwnd.get() as *mut core::ffi::c_void);
    let current_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let style = (current_style & !(WS_EX_APPWINDOW.0 as isize))
        | WS_EX_TOOLWINDOW.0 as isize
        | WS_EX_NOACTIVATE.0 as isize;
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        )?;
        let border_color = DWMWA_COLOR_NONE;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&border_color as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ) {
            log::debug!("failed to disable SSH Bridge notification window border: {error:?}");
        }
        let corner_preference: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner_preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of_val(&corner_preference) as u32,
        ) {
            log::debug!("failed to round SSH Bridge notification window: {error:?}");
        }

        // DWM can still draw a rectangular outline for an unowned tool window on
        // some Windows builds even when DWMWA_BORDER_COLOR is disabled. Give the
        // HWND an actual rounded region so that rectangular pixels do not belong
        // to the window at all. SetWindowRgn takes ownership after success.
        let mut bounds = RECT::default();
        if let Err(error) = GetWindowRect(hwnd, &mut bounds) {
            log::debug!("failed to query SSH Bridge notification bounds: {error:?}");
        } else {
            let width = bounds.right.saturating_sub(bounds.left);
            let height = bounds.bottom.saturating_sub(bounds.top);
            let corner_diameter = height.saturating_mul(40).checked_div(168).unwrap_or(40);
            let region = CreateRoundRectRgn(
                0,
                0,
                width.saturating_add(1),
                height.saturating_add(1),
                corner_diameter,
                corner_diameter,
            );
            if region.0.is_null() {
                log::debug!("failed to create SSH Bridge notification window region");
            } else if SetWindowRgn(hwnd, Some(region), true) == 0 {
                let _ = DeleteObject(HGDIOBJ(region.0));
                log::debug!("failed to apply SSH Bridge notification window region");
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn configure_notification_window(_window: &gpui::Window) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub async fn system_auth_available() -> bool {
    let (sender, receiver) = futures::channel::oneshot::channel();
    if std::thread::Builder::new()
        .name("miaominal-system-auth-check".into())
        .spawn(move || {
            let _ = sender.send(system_auth_available_blocking());
        })
        .is_err()
    {
        return false;
    }
    receiver.await.unwrap_or(false)
}

#[cfg(not(windows))]
pub async fn system_auth_available() -> bool {
    false
}

#[cfg(windows)]
pub async fn verify_system_auth(reason: &str) -> SystemAuthVerification {
    let reason = reason.to_string();
    let (sender, receiver) = futures::channel::oneshot::channel();
    if std::thread::Builder::new()
        .name("miaominal-system-auth".into())
        .spawn(move || {
            let _ = sender.send(verify_system_auth_blocking(&reason));
        })
        .is_err()
    {
        return SystemAuthVerification::Failed;
    }
    receiver.await.unwrap_or(SystemAuthVerification::Failed)
}

#[cfg(windows)]
fn system_auth_available_blocking() -> bool {
    use windows::Security::Credentials::UI::{
        UserConsentVerifier, UserConsentVerifierAvailability,
    };

    let Ok(_apartment) = enter_winrt_apartment() else {
        return false;
    };
    matches!(
        UserConsentVerifier::CheckAvailabilityAsync().and_then(|operation| operation.join()),
        Ok(UserConsentVerifierAvailability::Available)
    )
}

#[cfg(windows)]
fn verify_system_auth_blocking(reason: &str) -> SystemAuthVerification {
    use windows::Security::Credentials::UI::UserConsentVerifier;
    use windows::core::HSTRING;

    let Ok(_apartment) = enter_winrt_apartment() else {
        return SystemAuthVerification::Failed;
    };
    let Ok(operation) = UserConsentVerifier::RequestVerificationAsync(&HSTRING::from(reason))
    else {
        return SystemAuthVerification::Failed;
    };
    operation
        .join()
        .map(map_system_auth_result)
        .unwrap_or(SystemAuthVerification::Failed)
}

#[cfg(windows)]
fn map_system_auth_result(
    result: windows::Security::Credentials::UI::UserConsentVerificationResult,
) -> SystemAuthVerification {
    use windows::Security::Credentials::UI::UserConsentVerificationResult;

    if result == UserConsentVerificationResult::Verified {
        SystemAuthVerification::Verified
    } else if result == UserConsentVerificationResult::Canceled {
        SystemAuthVerification::Canceled
    } else if result == UserConsentVerificationResult::DeviceBusy {
        SystemAuthVerification::Busy
    } else if result == UserConsentVerificationResult::RetriesExhausted {
        SystemAuthVerification::RetriesExhausted
    } else if result == UserConsentVerificationResult::DeviceNotPresent
        || result == UserConsentVerificationResult::NotConfiguredForUser
        || result == UserConsentVerificationResult::DisabledByPolicy
    {
        SystemAuthVerification::Unavailable
    } else {
        SystemAuthVerification::Failed
    }
}

#[cfg(not(windows))]
pub async fn verify_system_auth(_reason: &str) -> SystemAuthVerification {
    SystemAuthVerification::Unavailable
}

#[cfg(windows)]
struct WinRtApartment {
    uninitialize: bool,
}

#[cfg(windows)]
impl Drop for WinRtApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { windows::Win32::System::WinRT::RoUninitialize() };
        }
    }
}

#[cfg(windows)]
fn enter_winrt_apartment() -> anyhow::Result<WinRtApartment> {
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => Ok(WinRtApartment { uninitialize: true }),
        Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(WinRtApartment {
            uninitialize: false,
        }),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn system_auth_results_fail_closed_except_verified() {
        use windows::Security::Credentials::UI::UserConsentVerificationResult;

        assert_eq!(
            map_system_auth_result(UserConsentVerificationResult::Verified),
            SystemAuthVerification::Verified
        );
        assert_eq!(
            map_system_auth_result(UserConsentVerificationResult::Canceled),
            SystemAuthVerification::Canceled
        );
        assert_eq!(
            map_system_auth_result(UserConsentVerificationResult::DeviceBusy),
            SystemAuthVerification::Busy
        );
        for result in [
            UserConsentVerificationResult::DeviceNotPresent,
            UserConsentVerificationResult::NotConfiguredForUser,
            UserConsentVerificationResult::DisabledByPolicy,
        ] {
            assert_eq!(
                map_system_auth_result(result),
                SystemAuthVerification::Unavailable
            );
        }
        assert_eq!(
            map_system_auth_result(UserConsentVerificationResult::RetriesExhausted),
            SystemAuthVerification::RetriesExhausted
        );
    }
}
