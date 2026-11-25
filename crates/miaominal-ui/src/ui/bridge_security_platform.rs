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
fn disable_non_client_frame_and_round_corners(
    hwnd: windows::Win32::Foundation::HWND,
    context_label: &str,
) {
    use windows::Win32::Graphics::Dwm::{
        DWM_WINDOW_CORNER_PREFERENCE, DWMNCRENDERINGPOLICY, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR,
        DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
        DwmSetWindowAttribute,
    };

    unsafe {
        let border_color = DWMWA_COLOR_NONE;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&border_color as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ) {
            log::debug!("failed to disable {context_label} window border: {error:?}");
        }
        // Win11 DWM can still draw an accent/highlight border on some builds even
        // when DWMWA_BORDER_COLOR is set to NONE. Disabling non-client rendering
        // entirely removes the compositor-layer outline. The rounded shape is
        // enforced by SetWindowRgn so nothing visual is lost.
        let ncr_policy = DWMNCRP_DISABLED;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            (&ncr_policy as *const DWMNCRENDERINGPOLICY).cast(),
            std::mem::size_of_val(&ncr_policy) as u32,
        ) {
            log::debug!("failed to disable {context_label} NC rendering: {error:?}");
        }
        let corner_preference: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner_preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of_val(&corner_preference) as u32,
        ) {
            log::debug!("failed to round {context_label} window: {error:?}");
        }
    }
}

#[cfg(windows)]
fn apply_rounded_window_region(
    hwnd: windows::Win32::Foundation::HWND,
    width: i32,
    height: i32,
    diameter: i32,
    context_label: &str,
) {
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, HGDIOBJ, SetWindowRgn};

    unsafe {
        let region = CreateRoundRectRgn(
            0,
            0,
            width.saturating_add(1),
            height.saturating_add(1),
            diameter,
            diameter,
        );
        if region.0.is_null() {
            log::debug!("failed to create {context_label} window region");
        } else if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(region.0));
            log::debug!("failed to apply {context_label} window region");
        }
    }
}

#[cfg(windows)]
fn scaled_region_diameter(physical_height: i32, logical_height: f32, corner_radius: f32) -> i32 {
    if logical_height > 0.0 && physical_height > 0 {
        ((physical_height as f32) * (corner_radius * 2.0 / logical_height)).round() as i32
    } else {
        (corner_radius * 2.0).round() as i32
    }
}

#[cfg(windows)]
pub fn configure_notification_window(window: &gpui_kit::Window) -> anyhow::Result<()> {
    use raw_window_handle::RawWindowHandle;
    use windows::Win32::Foundation::{HWND, RECT};
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
        disable_non_client_frame_and_round_corners(hwnd, "SSH Bridge notification");

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
            let corner_diameter = height.saturating_mul(40).checked_div(184).unwrap_or(40);
            apply_rounded_window_region(
                hwnd,
                width,
                height,
                corner_diameter,
                "SSH Bridge notification",
            );
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn configure_notification_window(_window: &gpui_kit::Window) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
#[derive(Debug)]
struct DragGhostNativeInner {
    hwnd: isize,
    alive: std::sync::atomic::AtomicBool,
    needs_frame_change: std::sync::atomic::AtomicBool,
    move_scheduled: std::sync::atomic::AtomicBool,
    grab_offset_x: std::sync::atomic::AtomicU32,
    grab_offset_y: std::sync::atomic::AtomicU32,
    last_physical_width: std::sync::atomic::AtomicI32,
    last_physical_height: std::sync::atomic::AtomicI32,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct DragGhostNativeHandle {
    inner: std::sync::Arc<DragGhostNativeInner>,
}

#[cfg(windows)]
impl DragGhostNativeHandle {
    pub fn invalidate(&self) {
        self.inner
            .alive
            .store(false, std::sync::atomic::Ordering::Release);
    }

    pub fn raw_hwnd(&self) -> Option<isize> {
        self.inner
            .alive
            .load(std::sync::atomic::Ordering::Acquire)
            .then_some(self.inner.hwnd)
    }

    #[cfg(test)]
    pub fn is_alive(&self) -> bool {
        self.inner.alive.load(std::sync::atomic::Ordering::Acquire)
    }

    #[cfg(test)]
    fn dummy_for_test() -> Self {
        Self {
            inner: std::sync::Arc::new(DragGhostNativeInner {
                hwnd: 0,
                alive: std::sync::atomic::AtomicBool::new(true),
                needs_frame_change: std::sync::atomic::AtomicBool::new(false),
                move_scheduled: std::sync::atomic::AtomicBool::new(false),
                grab_offset_x: std::sync::atomic::AtomicU32::new(0),
                grab_offset_y: std::sync::atomic::AtomicU32::new(0),
                last_physical_width: std::sync::atomic::AtomicI32::new(0),
                last_physical_height: std::sync::atomic::AtomicI32::new(0),
            }),
        }
    }
}

#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragGhostNativeHandle;

#[cfg(not(windows))]
impl DragGhostNativeHandle {
    pub fn invalidate(&self) {}

    pub fn raw_hwnd(&self) -> Option<isize> {
        None
    }
}

#[cfg(windows)]
pub fn topmost_window_under_cursor(
    candidate_hwnds: &[isize],
    ignored_hwnds: &[isize],
) -> Option<isize> {
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{
        GW_HWNDNEXT, GWL_EXSTYLE, GetCursorPos, GetTopWindow, GetWindow, GetWindowLongPtrW,
        GetWindowRect, IsIconic, IsWindowVisible, WS_EX_TRANSPARENT,
    };

    if candidate_hwnds.is_empty() {
        return None;
    }
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }.ok()?;

    let mut current: Option<HWND> = unsafe { GetTopWindow(None) }.ok();
    while let Some(hwnd) = current {
        let next = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.ok();
        let raw = hwnd.0 as isize;
        if ignored_hwnds.contains(&raw) {
            current = next;
            continue;
        }
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
            current = next;
            continue;
        }
        let mut cloaked: u32 = 0;
        let cloaked_ok = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                (&mut cloaked as *mut u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        }
        .is_ok();
        if cloaked_ok && cloaked != 0 {
            current = next;
            continue;
        }
        let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        if (ex_style & WS_EX_TRANSPARENT.0 as isize) != 0 {
            current = next;
            continue;
        }
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok()
            && cursor.x >= rect.left
            && cursor.x < rect.right
            && cursor.y >= rect.top
            && cursor.y < rect.bottom
        {
            return candidate_hwnds.contains(&raw).then_some(raw);
        }
        current = next;
    }
    None
}

#[cfg(not(windows))]
pub fn topmost_window_under_cursor(
    _candidate_hwnds: &[isize],
    _ignored_hwnds: &[isize],
) -> Option<isize> {
    None
}

#[cfg(windows)]
pub fn cursor_client_position(
    window: &gpui_kit::Window,
) -> Option<gpui_kit::Point<gpui_kit::Pixels>> {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let raw_hwnd = window_hwnd(window)?;
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt) }.ok()?;
    if !unsafe { ScreenToClient(HWND(raw_hwnd as *mut core::ffi::c_void), &mut pt) }.as_bool() {
        return None;
    }
    let scale = window.scale_factor().max(0.5);
    Some(gpui_kit::point(
        gpui_kit::px(pt.x as f32 / scale),
        gpui_kit::px(pt.y as f32 / scale),
    ))
}

#[cfg(not(windows))]
pub fn cursor_client_position(
    _window: &gpui_kit::Window,
) -> Option<gpui_kit::Point<gpui_kit::Pixels>> {
    None
}

pub fn supports_floating_drag_ghost(source_window: &gpui_kit::Window) -> bool {
    #[cfg(windows)]
    {
        window_hwnd(source_window).is_some()
    }
    #[cfg(not(windows))]
    {
        let _ = source_window;
        false
    }
}

#[cfg(windows)]
pub fn configure_drag_ghost_window(
    window: &gpui_kit::Window,
    grab_offset: gpui_kit::Point<gpui_kit::Pixels>,
    logical_height: f32,
    corner_radius: f32,
) -> anyhow::Result<DragGhostNativeHandle> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, AtomicU32},
    };
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    };

    let raw_hwnd = window_hwnd(window)
        .ok_or_else(|| anyhow::anyhow!("GPUI did not expose a Win32 drag ghost window handle"))?;
    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    let current_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let style = (current_style & !(WS_EX_APPWINDOW.0 as isize))
        | WS_EX_TOOLWINDOW.0 as isize
        | WS_EX_NOACTIVATE.0 as isize
        | WS_EX_TRANSPARENT.0 as isize
        | WS_EX_TOPMOST.0 as isize;
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        disable_non_client_frame_and_round_corners(hwnd, "tab drag ghost");
    }

    let mut rect = RECT::default();
    let (phys_w, phys_h) = if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        let phys_w = rect.right.saturating_sub(rect.left);
        let phys_h = rect.bottom.saturating_sub(rect.top);
        if phys_w > 0 && phys_h > 0 {
            let diameter = scaled_region_diameter(phys_h, logical_height, corner_radius);
            apply_rounded_window_region(hwnd, phys_w, phys_h, diameter, "tab drag ghost");
        }
        (phys_w, phys_h)
    } else {
        (0, 0)
    };

    // Defer SetWindowPos until after the active input dispatch finishes: calling
    // SetWindowPos synchronously here fires WM_MOVE while GPUI still holds a
    // mutable App borrow in on_input, causing re-entrant App borrow errors.
    Ok(DragGhostNativeHandle {
        inner: Arc::new(DragGhostNativeInner {
            hwnd: raw_hwnd,
            alive: AtomicBool::new(true),
            needs_frame_change: AtomicBool::new(true),
            move_scheduled: AtomicBool::new(false),
            grab_offset_x: AtomicU32::new(f32::from(grab_offset.x).to_bits()),
            grab_offset_y: AtomicU32::new(f32::from(grab_offset.y).to_bits()),
            last_physical_width: AtomicI32::new(phys_w),
            last_physical_height: AtomicI32::new(phys_h),
        }),
    })
}

#[cfg(not(windows))]
pub fn configure_drag_ghost_window(
    _window: &gpui_kit::Window,
    _grab_offset: gpui_kit::Point<gpui_kit::Pixels>,
    _logical_height: f32,
    _corner_radius: f32,
) -> anyhow::Result<DragGhostNativeHandle> {
    anyhow::bail!("floating drag ghost unsupported on this platform")
}

#[cfg(windows)]
pub fn schedule_drag_ghost_move(
    handle: &DragGhostNativeHandle,
    grab_offset: gpui_kit::Point<gpui_kit::Pixels>,
    logical_height: f32,
    corner_radius: f32,
    cx: &gpui_kit::App,
) {
    use std::sync::{Arc, atomic::Ordering};
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, HWND_TOPMOST, IsWindow, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
    };

    if !handle.inner.alive.load(Ordering::Acquire) {
        return;
    }
    handle
        .inner
        .grab_offset_x
        .store(f32::from(grab_offset.x).to_bits(), Ordering::Relaxed);
    handle
        .inner
        .grab_offset_y
        .store(f32::from(grab_offset.y).to_bits(), Ordering::Relaxed);
    if handle.inner.move_scheduled.swap(true, Ordering::AcqRel) {
        return;
    }

    let inner = Arc::clone(&handle.inner);
    cx.foreground_executor()
        .spawn(async move {
            inner.move_scheduled.store(false, Ordering::Release);
            if !inner.alive.load(Ordering::Acquire) {
                return;
            }
            let hwnd = HWND(inner.hwnd as *mut core::ffi::c_void);
            if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                inner.alive.store(false, Ordering::Release);
                return;
            }

            let mut rect = RECT::default();
            if let Err(error) = unsafe { GetWindowRect(hwnd, &mut rect) } {
                log::debug!("failed to query drag ghost window bounds: {error:?}");
                return;
            }
            let phys_w = rect.right.saturating_sub(rect.left);
            let phys_h = rect.bottom.saturating_sub(rect.top);
            let prev_w = inner.last_physical_width.load(Ordering::Relaxed);
            let prev_h = inner.last_physical_height.load(Ordering::Relaxed);
            if (phys_w, phys_h) != (prev_w, prev_h) && phys_w > 0 && phys_h > 0 {
                let diameter = scaled_region_diameter(phys_h, logical_height, corner_radius);
                apply_rounded_window_region(hwnd, phys_w, phys_h, diameter, "tab drag ghost");
                inner.last_physical_width.store(phys_w, Ordering::Relaxed);
                inner.last_physical_height.store(phys_h, Ordering::Relaxed);
            }

            let scale = if logical_height > 0.0 && phys_h > 0 {
                (phys_h as f32 / logical_height).max(0.5)
            } else {
                1.0
            };
            let mut cursor = POINT::default();
            if let Err(error) = unsafe { GetCursorPos(&mut cursor) } {
                log::debug!("failed to query cursor position for drag ghost: {error:?}");
                return;
            }
            let offset_x = f32::from_bits(inner.grab_offset_x.load(Ordering::Relaxed));
            let offset_y = f32::from_bits(inner.grab_offset_y.load(Ordering::Relaxed));
            let x = cursor.x - (offset_x * scale).round() as i32;
            let y = cursor.y - (offset_y * scale).round() as i32;

            if !inner.alive.load(Ordering::Acquire) || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                return;
            }
            let mut flags = SWP_NOACTIVATE | SWP_NOSIZE | SWP_SHOWWINDOW;
            if inner.needs_frame_change.swap(false, Ordering::AcqRel) {
                flags |= SWP_FRAMECHANGED;
            }
            if let Err(error) = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, 0, 0, flags) }
            {
                log::debug!("failed to move drag ghost window: {error:?}");
            }
        })
        .detach();
}

#[cfg(not(windows))]
pub fn schedule_drag_ghost_move(
    _handle: &DragGhostNativeHandle,
    _grab_offset: gpui_kit::Point<gpui_kit::Pixels>,
    _logical_height: f32,
    _corner_radius: f32,
    _cx: &gpui_kit::App,
) {
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
pub fn window_hwnd(window: &gpui_kit::Window) -> Option<isize> {
    use raw_window_handle::RawWindowHandle;

    let handle = raw_window_handle::HasWindowHandle::window_handle(window)
        .ok()?
        .as_raw();
    match handle {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn window_hwnd(_window: &gpui_kit::Window) -> Option<isize> {
    None
}

#[cfg(windows)]
pub async fn verify_system_auth(reason: &str, owner_hwnd: Option<isize>) -> SystemAuthVerification {
    let reason = reason.to_string();
    let (sender, receiver) = futures::channel::oneshot::channel();
    if std::thread::Builder::new()
        .name("miaominal-system-auth".into())
        .spawn(move || {
            let _ = sender.send(verify_system_auth_blocking(&reason, owner_hwnd));
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
fn request_verification_for_window<T: windows::core::Interface>(
    _marker: fn(&windows::core::HSTRING) -> windows::core::Result<T>,
    hwnd: windows::Win32::Foundation::HWND,
    reason: &windows::core::HSTRING,
) -> windows::core::Result<T> {
    use windows::Security::Credentials::UI::UserConsentVerifier;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;

    let interop: IUserConsentVerifierInterop =
        windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
    unsafe { interop.RequestVerificationForWindowAsync::<T>(hwnd, reason) }
}

#[cfg(windows)]
fn verify_system_auth_blocking(reason: &str, owner_hwnd: Option<isize>) -> SystemAuthVerification {
    use windows::Security::Credentials::UI::UserConsentVerifier;
    use windows::Win32::Foundation::HWND;
    use windows::core::HSTRING;

    let Ok(_apartment) = enter_winrt_apartment() else {
        return SystemAuthVerification::Failed;
    };
    let message = HSTRING::from(reason);
    let operation = match owner_hwnd.filter(|hwnd| *hwnd != 0) {
        Some(raw_hwnd) => {
            let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
            match request_verification_for_window(
                UserConsentVerifier::RequestVerificationAsync,
                hwnd,
                &message,
            ) {
                Ok(operation) => Ok(operation),
                Err(error) => {
                    log::debug!(
                        "IUserConsentVerifierInterop::RequestVerificationForWindowAsync failed, falling back: {error:?}"
                    );
                    UserConsentVerifier::RequestVerificationAsync(&message)
                }
            }
        }
        None => UserConsentVerifier::RequestVerificationAsync(&message),
    };
    let Ok(operation) = operation else {
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
pub async fn verify_system_auth(
    _reason: &str,
    _owner_hwnd: Option<isize>,
) -> SystemAuthVerification {
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

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

    #[test]
    fn drag_ghost_native_handle_invalidate_marks_dead() {
        let handle = DragGhostNativeHandle::dummy_for_test();
        assert!(handle.is_alive());
        handle.invalidate();
        assert!(!handle.is_alive());
    }
}
