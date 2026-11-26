#[derive(Clone, Debug, PartialEq)]
pub struct DragGhostVisualSpec {
    pub title: String,
    pub logical_width: f32,
    pub logical_height: f32,
    pub corner_radius: f32,
    pub background_rgb: u32,
    pub background_alpha: f32,
    pub text_rgb: u32,
    pub border_rgb: Option<u32>,
    pub status_rgb: Option<u32>,
    pub detach_plus_rgb: Option<u32>,
}

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
fn disable_window_composition_accent(hwnd: windows::Win32::Foundation::HWND) {
    use core::ffi::c_void;

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }

    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: u32,
        pv_data: *mut c_void,
        cb_data: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleA(lp_module_name: *const u8) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *const c_void;
    }

    type SetWindowCompositionAttributeFn = unsafe extern "system" fn(
        windows::Win32::Foundation::HWND,
        *mut WindowCompositionAttribData,
    ) -> i32;

    const WCA_ACCENT_POLICY: u32 = 19;

    unsafe {
        let user32 = GetModuleHandleA(c"user32.dll".as_ptr().cast());
        if user32.is_null() {
            return;
        }
        let proc = GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast());
        if proc.is_null() {
            return;
        }
        let set_window_composition_attribute: SetWindowCompositionAttributeFn =
            std::mem::transmute(proc);
        let mut accent = AccentPolicy {
            accent_state: 0,
            accent_flags: 0,
            gradient_color: 0,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            pv_data: (&mut accent as *mut AccentPolicy).cast(),
            cb_data: std::mem::size_of::<AccentPolicy>(),
        };
        let _ = set_window_composition_attribute(hwnd, &mut data);
    }
}

#[cfg(windows)]
fn configure_transparent_drag_ghost_dwm(
    hwnd: windows::Win32::Foundation::HWND,
    uses_direct_composition: bool,
) {
    use windows::Win32::Graphics::Dwm::{
        DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMNCRENDERINGPOLICY,
        DWMNCRP_DISABLED, DWMSBT_NONE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_NCRENDERING_POLICY, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_TRANSITIONS_FORCEDISABLED,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DwmSetWindowAttribute,
    };

    unsafe {
        let border_color = DWMWA_COLOR_NONE;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&border_color as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ) {
            log::debug!("failed to disable tab drag ghost window border: {error:?}");
        }
        let ncr_policy = DWMNCRP_DISABLED;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            (&ncr_policy as *const DWMNCRENDERINGPOLICY).cast(),
            std::mem::size_of_val(&ncr_policy) as u32,
        ) {
            log::debug!("failed to disable tab drag ghost NC rendering: {error:?}");
        }
        let disable_transitions: u32 = 1;
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            (&disable_transitions as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ) {
            log::debug!("failed to disable tab drag ghost transitions: {error:?}");
        }
        let backdrop: DWM_SYSTEMBACKDROP_TYPE = DWMSBT_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&backdrop as *const DWM_SYSTEMBACKDROP_TYPE).cast(),
            std::mem::size_of_val(&backdrop) as u32,
        );
        let corner_preference: DWM_WINDOW_CORNER_PREFERENCE = if uses_direct_composition {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_ROUND
        };
        if let Err(error) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner_preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
            std::mem::size_of_val(&corner_preference) as u32,
        ) {
            log::debug!("failed to configure tab drag ghost corners: {error:?}");
        }
    }
    if uses_direct_composition {
        disable_window_composition_accent(hwnd);
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
    uses_direct_composition: bool,
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
                uses_direct_composition: true,
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

#[cfg(target_os = "linux")]
mod linux_dl {
    use core::ffi::{CStr, c_char, c_int, c_void};

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    const RTLD_LAZY: c_int = 1;

    pub(super) unsafe fn open_any(names: &[&CStr]) -> *mut c_void {
        for name in names {
            let handle = unsafe { dlopen(name.as_ptr(), RTLD_LAZY) };
            if !handle.is_null() {
                return handle;
            }
        }
        core::ptr::null_mut()
    }

    pub(super) unsafe fn sym(handle: *mut c_void, name: &CStr) -> *mut c_void {
        if handle.is_null() {
            return core::ptr::null_mut();
        }
        unsafe { dlsym(handle, name.as_ptr()) }
    }
}

#[cfg(target_os = "linux")]
mod linux_xcb {
    use super::linux_dl;
    use core::ffi::{c_int, c_void};
    use std::sync::OnceLock;

    #[repr(C)]
    pub(super) struct XcbVoidCookie {
        pub(super) sequence: u32,
    }

    pub(super) type ConfigureWindowFn = unsafe extern "C" fn(
        conn: *mut c_void,
        window: u32,
        value_mask: u16,
        value_list: *const c_void,
    ) -> XcbVoidCookie;

    pub(super) type FlushFn = unsafe extern "C" fn(conn: *mut c_void) -> c_int;

    pub(super) type ShapeRectanglesFn = unsafe extern "C" fn(
        conn: *mut c_void,
        operation: u8,
        destination_kind: u8,
        ordering: u8,
        destination_window: u32,
        x_offset: i16,
        y_offset: i16,
        rectangles_len: u32,
        rectangles: *const c_void,
    ) -> XcbVoidCookie;

    pub(super) struct LinuxXcbApi {
        pub(super) configure_window: ConfigureWindowFn,
        pub(super) flush: FlushFn,
        pub(super) shape_rectangles: Option<ShapeRectanglesFn>,
    }

    pub(super) fn api() -> Option<&'static LinuxXcbApi> {
        static API: OnceLock<Option<LinuxXcbApi>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let xcb = linux_dl::open_any(&[c"libxcb.so.1", c"libxcb.so"]);
            let configure_ptr = linux_dl::sym(xcb, c"xcb_configure_window");
            let flush_ptr = linux_dl::sym(xcb, c"xcb_flush");
            if configure_ptr.is_null() || flush_ptr.is_null() {
                return None;
            }
            let xcb_shape = linux_dl::open_any(&[c"libxcb-shape.so.0", c"libxcb-shape.so"]);
            let shape_ptr = linux_dl::sym(xcb_shape, c"xcb_shape_rectangles");
            Some(LinuxXcbApi {
                configure_window: core::mem::transmute(configure_ptr),
                flush: core::mem::transmute(flush_ptr),
                shape_rectangles: if shape_ptr.is_null() {
                    None
                } else {
                    Some(core::mem::transmute(shape_ptr))
                },
            })
        })
        .as_ref()
    }
}

#[cfg(target_os = "linux")]
mod linux_cairo {
    use super::DragGhostVisualSpec;
    use super::linux_dl;
    use core::ffi::{c_char, c_double, c_int, c_void};
    use std::ffi::CString;
    use std::sync::OnceLock;

    const CAIRO_FORMAT_ARGB32: c_int = 0;
    const CAIRO_LINE_CAP_ROUND: c_int = 1;
    const PANGO_SCALE: c_double = 1024.0;
    const PANGO_ELLIPSIZE_END: c_int = 3;

    type ImageSurfaceCreateForDataFn =
        unsafe extern "C" fn(*mut u8, c_int, c_int, c_int, c_int) -> *mut c_void;
    type SurfaceSetDeviceScaleFn = unsafe extern "C" fn(*mut c_void, c_double, c_double);
    type SurfaceFlushFn = unsafe extern "C" fn(*mut c_void);
    type SurfaceDestroyFn = unsafe extern "C" fn(*mut c_void);
    type CairoCreateFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    type CairoDestroyFn = unsafe extern "C" fn(*mut c_void);
    type CairoSetSourceRgbaFn =
        unsafe extern "C" fn(*mut c_void, c_double, c_double, c_double, c_double);
    type CairoNewSubPathFn = unsafe extern "C" fn(*mut c_void);
    type CairoArcFn =
        unsafe extern "C" fn(*mut c_void, c_double, c_double, c_double, c_double, c_double);
    type CairoClosePathFn = unsafe extern "C" fn(*mut c_void);
    type CairoFillFn = unsafe extern "C" fn(*mut c_void);
    type CairoStrokeFn = unsafe extern "C" fn(*mut c_void);
    type CairoSetLineWidthFn = unsafe extern "C" fn(*mut c_void, c_double);
    type CairoSetLineCapFn = unsafe extern "C" fn(*mut c_void, c_int);
    type CairoMoveToFn = unsafe extern "C" fn(*mut c_void, c_double, c_double);
    type CairoLineToFn = unsafe extern "C" fn(*mut c_void, c_double, c_double);
    type PangoCairoCreateLayoutFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    type PangoCairoShowLayoutFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
    type PangoFontDescNewFn = unsafe extern "C" fn() -> *mut c_void;
    type PangoFontDescSetFamilyFn = unsafe extern "C" fn(*mut c_void, *const c_char);
    type PangoFontDescSetAbsoluteSizeFn = unsafe extern "C" fn(*mut c_void, c_double);
    type PangoFontDescFreeFn = unsafe extern "C" fn(*mut c_void);
    type PangoLayoutSetFontDescFn = unsafe extern "C" fn(*mut c_void, *const c_void);
    type PangoLayoutSetTextFn = unsafe extern "C" fn(*mut c_void, *const c_char, c_int);
    type PangoLayoutSetWidthFn = unsafe extern "C" fn(*mut c_void, c_int);
    type PangoLayoutSetEllipsizeFn = unsafe extern "C" fn(*mut c_void, c_int);
    type PangoLayoutGetPixelSizeFn = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int);
    type GObjectUnrefFn = unsafe extern "C" fn(*mut c_void);

    struct CairoPangoApi {
        image_surface_create_for_data: ImageSurfaceCreateForDataFn,
        surface_set_device_scale: SurfaceSetDeviceScaleFn,
        surface_flush: SurfaceFlushFn,
        surface_destroy: SurfaceDestroyFn,
        create: CairoCreateFn,
        destroy: CairoDestroyFn,
        set_source_rgba: CairoSetSourceRgbaFn,
        new_sub_path: CairoNewSubPathFn,
        arc: CairoArcFn,
        close_path: CairoClosePathFn,
        fill: CairoFillFn,
        stroke: CairoStrokeFn,
        set_line_width: CairoSetLineWidthFn,
        set_line_cap: CairoSetLineCapFn,
        move_to: CairoMoveToFn,
        line_to: CairoLineToFn,
        pango_cairo_create_layout: PangoCairoCreateLayoutFn,
        pango_cairo_show_layout: PangoCairoShowLayoutFn,
        pango_font_desc_new: PangoFontDescNewFn,
        pango_font_desc_set_family: PangoFontDescSetFamilyFn,
        pango_font_desc_set_absolute_size: PangoFontDescSetAbsoluteSizeFn,
        pango_font_desc_free: PangoFontDescFreeFn,
        pango_layout_set_font_desc: PangoLayoutSetFontDescFn,
        pango_layout_set_text: PangoLayoutSetTextFn,
        pango_layout_set_width: PangoLayoutSetWidthFn,
        pango_layout_set_ellipsize: PangoLayoutSetEllipsizeFn,
        pango_layout_get_pixel_size: PangoLayoutGetPixelSizeFn,
        g_object_unref: GObjectUnrefFn,
    }

    fn api() -> Option<&'static CairoPangoApi> {
        static API: OnceLock<Option<CairoPangoApi>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let cairo = linux_dl::open_any(&[c"libcairo.so.2", c"libcairo.so"]);
            let pangocairo =
                linux_dl::open_any(&[c"libpangocairo-1.0.so.0", c"libpangocairo-1.0.so"]);
            let pango = linux_dl::open_any(&[c"libpango-1.0.so.0", c"libpango-1.0.so"]);
            let gobject = linux_dl::open_any(&[c"libgobject-2.0.so.0", c"libgobject-2.0.so"]);
            if cairo.is_null() || pangocairo.is_null() || pango.is_null() || gobject.is_null() {
                return None;
            }
            macro_rules! load {
                ($lib:expr, $name:expr) => {{
                    let ptr = linux_dl::sym($lib, $name);
                    if ptr.is_null() {
                        return None;
                    }
                    core::mem::transmute(ptr)
                }};
            }
            Some(CairoPangoApi {
                image_surface_create_for_data: load!(cairo, c"cairo_image_surface_create_for_data"),
                surface_set_device_scale: load!(cairo, c"cairo_surface_set_device_scale"),
                surface_flush: load!(cairo, c"cairo_surface_flush"),
                surface_destroy: load!(cairo, c"cairo_surface_destroy"),
                create: load!(cairo, c"cairo_create"),
                destroy: load!(cairo, c"cairo_destroy"),
                set_source_rgba: load!(cairo, c"cairo_set_source_rgba"),
                new_sub_path: load!(cairo, c"cairo_new_sub_path"),
                arc: load!(cairo, c"cairo_arc"),
                close_path: load!(cairo, c"cairo_close_path"),
                fill: load!(cairo, c"cairo_fill"),
                stroke: load!(cairo, c"cairo_stroke"),
                set_line_width: load!(cairo, c"cairo_set_line_width"),
                set_line_cap: load!(cairo, c"cairo_set_line_cap"),
                move_to: load!(cairo, c"cairo_move_to"),
                line_to: load!(cairo, c"cairo_line_to"),
                pango_cairo_create_layout: load!(pangocairo, c"pango_cairo_create_layout"),
                pango_cairo_show_layout: load!(pangocairo, c"pango_cairo_show_layout"),
                pango_font_desc_new: load!(pango, c"pango_font_description_new"),
                pango_font_desc_set_family: load!(pango, c"pango_font_description_set_family"),
                pango_font_desc_set_absolute_size: load!(
                    pango,
                    c"pango_font_description_set_absolute_size"
                ),
                pango_font_desc_free: load!(pango, c"pango_font_description_free"),
                pango_layout_set_font_desc: load!(pango, c"pango_layout_set_font_description"),
                pango_layout_set_text: load!(pango, c"pango_layout_set_text"),
                pango_layout_set_width: load!(pango, c"pango_layout_set_width"),
                pango_layout_set_ellipsize: load!(pango, c"pango_layout_set_ellipsize"),
                pango_layout_get_pixel_size: load!(pango, c"pango_layout_get_pixel_size"),
                g_object_unref: load!(gobject, c"g_object_unref"),
            })
        })
        .as_ref()
    }

    fn rgb_f64(rgb: u32) -> (c_double, c_double, c_double) {
        (
            ((rgb >> 16) & 0xff) as c_double / 255.0,
            ((rgb >> 8) & 0xff) as c_double / 255.0,
            (rgb & 0xff) as c_double / 255.0,
        )
    }

    unsafe fn rounded_rect_path(
        api: &CairoPangoApi,
        cr: *mut c_void,
        x: c_double,
        y: c_double,
        w: c_double,
        h: c_double,
        r: c_double,
    ) {
        let radius = r.min(w * 0.5).min(h * 0.5).max(0.0);
        let right = x + w;
        let bottom = y + h;
        let half_pi = core::f64::consts::FRAC_PI_2;
        unsafe {
            (api.new_sub_path)(cr);
            (api.arc)(cr, right - radius, y + radius, radius, -half_pi, 0.0);
            (api.arc)(cr, right - radius, bottom - radius, radius, 0.0, half_pi);
            (api.arc)(
                cr,
                x + radius,
                bottom - radius,
                radius,
                half_pi,
                core::f64::consts::PI,
            );
            (api.arc)(
                cr,
                x + radius,
                y + radius,
                radius,
                core::f64::consts::PI,
                core::f64::consts::PI + half_pi,
            );
            (api.close_path)(cr);
        }
    }

    fn render_fallback_pill(
        visual: &DragGhostVisualSpec,
        px_w: i32,
        px_h: i32,
        scale: i32,
        pixels: &mut [u8],
    ) {
        pixels.fill(0);
        let w = visual.logical_width.max(1.0);
        let h = visual.logical_height.max(1.0);
        let r = visual.corner_radius.min(w * 0.5).min(h * 0.5);
        let scale_f = scale.max(1) as f32;
        let alpha = visual.background_alpha.clamp(0.0, 1.0);
        let half_w = w * 0.5;
        let half_h = h * 0.5;
        let inner_x = (half_w - r).max(0.0);
        let inner_y = (half_h - r).max(0.0);
        let stride = px_w as usize * 4;

        let blend_premul = |dst: &mut [u8], rgb: u32, cov: f32| {
            let src_a = cov.clamp(0.0, 1.0);
            if src_a <= 0.0 {
                return;
            }
            let sr = ((rgb >> 16) & 0xff) as f32 / 255.0;
            let sg = ((rgb >> 8) & 0xff) as f32 / 255.0;
            let sb = (rgb & 0xff) as f32 / 255.0;
            let inv = 1.0 - src_a;
            let dr = dst[2] as f32 / 255.0;
            let dg = dst[1] as f32 / 255.0;
            let db = dst[0] as f32 / 255.0;
            let da = dst[3] as f32 / 255.0;
            dst[2] = ((sr * src_a + dr * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
            dst[1] = ((sg * src_a + dg * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
            dst[0] = ((sb * src_a + db * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
            dst[3] = ((src_a + da * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
        };

        for py in 0..px_h.max(0) as usize {
            let ly = (py as f32 + 0.5) / scale_f;
            let dy = (ly - half_h).abs() - inner_y;
            for px_i in 0..px_w.max(0) as usize {
                let lx = (px_i as f32 + 0.5) / scale_f;
                let dx = (lx - half_w).abs() - inner_x;
                let dist = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
                let aa_cov = (0.5 - dist * scale_f).clamp(0.0, 1.0);
                if aa_cov <= 0.0 {
                    continue;
                }
                let cell = &mut pixels[py * stride + px_i * 4..py * stride + px_i * 4 + 4];
                blend_premul(cell, visual.background_rgb, alpha * aa_cov);
                if let Some(border_rgb) = visual.border_rgb {
                    let edge_dist = (dist + 0.5).abs();
                    let border_cov = (1.0 - edge_dist * scale_f).clamp(0.0, 1.0);
                    blend_premul(cell, border_rgb, alpha * border_cov);
                }
                if let Some(status_rgb) = visual.status_rgb {
                    let dot_dist = (lx - 15.5).hypot(ly - half_h) - 3.5;
                    let dot_cov = (0.5 - dot_dist * scale_f).clamp(0.0, 1.0);
                    blend_premul(cell, status_rgb, alpha * dot_cov);
                }
                if let Some(plus_rgb) = visual.detach_plus_rgb {
                    let cx = w - 19.0;
                    let cy = half_h;
                    let h_bar = ((lx - cx).abs() - 4.0).max((ly - cy).abs() - 0.8);
                    let v_bar = ((lx - cx).abs() - 0.8).max((ly - cy).abs() - 4.0);
                    let plus_cov = (0.5 - h_bar.min(v_bar) * scale_f).clamp(0.0, 1.0);
                    blend_premul(cell, plus_rgb, alpha * plus_cov);
                }
            }
        }
    }

    pub(super) fn render_pill_argb8888(
        visual: &DragGhostVisualSpec,
        px_w: i32,
        px_h: i32,
        stride: i32,
        scale: i32,
        pixels: &mut [u8],
    ) {
        let Some(api) = api() else {
            render_fallback_pill(visual, px_w, px_h, scale, pixels);
            return;
        };
        pixels.fill(0);
        let w = visual.logical_width.max(1.0) as c_double;
        let h = visual.logical_height.max(1.0) as c_double;
        let r = visual.corner_radius as c_double;
        let alpha = visual.background_alpha.clamp(0.0, 1.0) as c_double;
        unsafe {
            let surface = (api.image_surface_create_for_data)(
                pixels.as_mut_ptr(),
                CAIRO_FORMAT_ARGB32,
                px_w,
                px_h,
                stride,
            );
            if surface.is_null() {
                render_fallback_pill(visual, px_w, px_h, scale, pixels);
                return;
            }
            if scale > 1 {
                (api.surface_set_device_scale)(surface, scale as c_double, scale as c_double);
            }
            let cr = (api.create)(surface);
            if cr.is_null() {
                (api.surface_destroy)(surface);
                render_fallback_pill(visual, px_w, px_h, scale, pixels);
                return;
            }

            let (bg_r, bg_g, bg_b) = rgb_f64(visual.background_rgb);
            (api.set_source_rgba)(cr, bg_r, bg_g, bg_b, alpha);
            rounded_rect_path(api, cr, 0.0, 0.0, w, h, r);
            (api.fill)(cr);

            if let Some(border_rgb) = visual.border_rgb {
                let (br, bg, bb) = rgb_f64(border_rgb);
                (api.set_source_rgba)(cr, br, bg, bb, alpha);
                (api.set_line_width)(cr, 1.0);
                rounded_rect_path(
                    api,
                    cr,
                    0.5,
                    0.5,
                    (w - 1.0).max(1.0),
                    (h - 1.0).max(1.0),
                    (r - 0.5).max(0.0),
                );
                (api.stroke)(cr);
            }

            let mut text_left = 12.0;
            if let Some(status_rgb) = visual.status_rgb {
                let (sr, sg, sb) = rgb_f64(status_rgb);
                (api.set_source_rgba)(cr, sr, sg, sb, alpha);
                (api.new_sub_path)(cr);
                (api.arc)(
                    cr,
                    text_left + 3.5,
                    h * 0.5,
                    3.5,
                    0.0,
                    core::f64::consts::TAU,
                );
                (api.fill)(cr);
                text_left += 15.0;
            }

            let mut text_right_reserved = 12.0;
            if let Some(plus_rgb) = visual.detach_plus_rgb {
                let (pr, pg, pb) = rgb_f64(plus_rgb);
                let cx = w - 12.0 - 7.0;
                let cy = h * 0.5;
                let arm = 4.2;
                (api.set_source_rgba)(cr, pr, pg, pb, alpha);
                (api.set_line_width)(cr, 1.6);
                (api.set_line_cap)(cr, CAIRO_LINE_CAP_ROUND);
                (api.move_to)(cr, cx - arm, cy);
                (api.line_to)(cr, cx + arm, cy);
                (api.move_to)(cr, cx, cy - arm);
                (api.line_to)(cr, cx, cy + arm);
                (api.stroke)(cr);
                text_right_reserved += 22.0;
            }

            let max_text_w = (w - text_left - text_right_reserved).max(8.0);
            let title_clean = visual.title.replace('\0', "");
            if let Ok(title_cstr) = CString::new(title_clean) {
                let layout = (api.pango_cairo_create_layout)(cr);
                if !layout.is_null() {
                    let desc = (api.pango_font_desc_new)();
                    if !desc.is_null() {
                        (api.pango_font_desc_set_family)(desc, c"sans-serif".as_ptr());
                        (api.pango_font_desc_set_absolute_size)(desc, 13.5 * PANGO_SCALE);
                        (api.pango_layout_set_font_desc)(layout, desc);
                        (api.pango_font_desc_free)(desc);
                    }
                    (api.pango_layout_set_width)(
                        layout,
                        (max_text_w * PANGO_SCALE).round() as c_int,
                    );
                    (api.pango_layout_set_ellipsize)(layout, PANGO_ELLIPSIZE_END);
                    (api.pango_layout_set_text)(
                        layout,
                        title_cstr.as_ptr(),
                        title_cstr.as_bytes().len() as c_int,
                    );
                    let mut text_w: c_int = 0;
                    let mut text_h: c_int = 0;
                    (api.pango_layout_get_pixel_size)(layout, &mut text_w, &mut text_h);
                    let (tr, tg, tb) = rgb_f64(visual.text_rgb);
                    (api.set_source_rgba)(cr, tr, tg, tb, alpha);
                    (api.move_to)(cr, text_left, (h - text_h as c_double) * 0.5);
                    (api.pango_cairo_show_layout)(cr, layout);
                    (api.g_object_unref)(layout);
                }
            }

            (api.destroy)(cr);
            (api.surface_flush)(surface);
            (api.surface_destroy)(surface);
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_wayland {
    use super::DragGhostVisualSpec;
    use super::linux_cairo;
    use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{
        wl_buffer, wl_compositor, wl_region, wl_registry, wl_shm, wl_shm_pool, wl_subcompositor,
        wl_subsurface, wl_surface,
    };
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};

    struct WaylandGhostState;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandGhostState {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(WaylandGhostState: ignore wl_compositor::WlCompositor);
    delegate_noop!(WaylandGhostState: ignore wl_subcompositor::WlSubcompositor);
    delegate_noop!(WaylandGhostState: ignore wl_shm::WlShm);
    delegate_noop!(WaylandGhostState: ignore wl_shm_pool::WlShmPool);
    delegate_noop!(WaylandGhostState: ignore wl_buffer::WlBuffer);
    delegate_noop!(WaylandGhostState: ignore wl_surface::WlSurface);
    delegate_noop!(WaylandGhostState: ignore wl_subsurface::WlSubsurface);
    delegate_noop!(WaylandGhostState: ignore wl_region::WlRegion);

    struct ShmBuffer {
        buffer: wl_buffer::WlBuffer,
        pool: wl_shm_pool::WlShmPool,
        ptr: *mut libc::c_void,
        len: usize,
    }

    impl ShmBuffer {
        fn allocate(
            shm: &wl_shm::WlShm,
            qh: &QueueHandle<WaylandGhostState>,
            visual: &DragGhostVisualSpec,
            scale: i32,
        ) -> anyhow::Result<Self> {
            let log_w = visual.logical_width.round().max(1.0) as i32;
            let log_h = visual.logical_height.round().max(1.0) as i32;
            let scale = scale.max(1);
            let px_w = log_w.saturating_mul(scale);
            let px_h = log_h.saturating_mul(scale);
            let stride = px_w.saturating_mul(4);
            let len = (stride as usize).saturating_mul(px_h as usize);

            let fd = unsafe {
                let raw_fd =
                    libc::memfd_create(c"miaominal-drag-ghost".as_ptr(), libc::MFD_CLOEXEC);
                if raw_fd < 0 {
                    anyhow::bail!("memfd_create failed for Wayland drag ghost");
                }
                let owned = OwnedFd::from_raw_fd(raw_fd);
                if libc::ftruncate(owned.as_raw_fd(), len as libc::off_t) != 0 {
                    anyhow::bail!("ftruncate failed for Wayland drag ghost");
                }
                owned
            };
            let ptr = unsafe {
                libc::mmap(
                    core::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd.as_raw_fd(),
                    0,
                )
            };
            if ptr == libc::MAP_FAILED {
                anyhow::bail!("mmap failed for Wayland drag ghost");
            }

            {
                let pixels = unsafe { core::slice::from_raw_parts_mut(ptr.cast::<u8>(), len) };
                linux_cairo::render_pill_argb8888(visual, px_w, px_h, stride, scale, pixels);
            }

            let pool = shm.create_pool(fd.as_fd(), len as i32, qh, ());
            let buffer =
                pool.create_buffer(0, px_w, px_h, stride, wl_shm::Format::Argb8888, qh, ());
            Ok(Self {
                buffer,
                pool,
                ptr,
                len,
            })
        }

        fn destroy(&mut self) {
            self.buffer.destroy();
            self.pool.destroy();
            if !self.ptr.is_null() && self.len > 0 {
                unsafe {
                    let _ = libc::munmap(self.ptr, self.len);
                }
                self.ptr = core::ptr::null_mut();
                self.len = 0;
            }
        }
    }

    pub(super) struct WaylandGhostSession {
        conn: Connection,
        _queue: EventQueue<WaylandGhostState>,
        qh: QueueHandle<WaylandGhostState>,
        shm: wl_shm::WlShm,
        parent_surface: wl_surface::WlSurface,
        child_surface: wl_surface::WlSurface,
        subsurface: wl_subsurface::WlSubsurface,
        buffer_scale: i32,
        buffers: Vec<ShmBuffer>,
        closed: bool,
    }

    // `WaylandGhostSession` only accesses libwayland on the foreground UI executor
    // thread alongside GPUI's Wayland display connection.
    unsafe impl Send for WaylandGhostSession {}

    impl WaylandGhostSession {
        pub(super) fn open(
            surface_ptr: usize,
            display_ptr: usize,
            local_x: i32,
            local_y: i32,
            scale_factor: f32,
            visual: &DragGhostVisualSpec,
        ) -> anyhow::Result<Self> {
            let backend = unsafe {
                Backend::from_foreign_display(display_ptr as *mut wayland_sys::client::wl_display)
            };
            let conn = Connection::from_backend(backend);
            let (globals, queue) = registry_queue_init::<WaylandGhostState>(&conn)
                .map_err(|error| anyhow::anyhow!("Wayland registry init failed: {error:?}"))?;
            let qh = queue.handle();

            let compositor: wl_compositor::WlCompositor = globals
                .bind(&qh, 1..=6, ())
                .map_err(|error| anyhow::anyhow!("wl_compositor unavailable: {error:?}"))?;
            let subcompositor: wl_subcompositor::WlSubcompositor = globals
                .bind(&qh, 1..=1, ())
                .map_err(|error| anyhow::anyhow!("wl_subcompositor unavailable: {error:?}"))?;
            let shm: wl_shm::WlShm = globals
                .bind(&qh, 1..=2, ())
                .map_err(|error| anyhow::anyhow!("wl_shm unavailable: {error:?}"))?;

            let parent_id = unsafe {
                ObjectId::from_ptr(
                    wl_surface::WlSurface::interface(),
                    surface_ptr as *mut wayland_sys::client::wl_proxy,
                )
            }
            .map_err(|error| anyhow::anyhow!("invalid parent wl_surface handle: {error:?}"))?;
            let parent_surface = wl_surface::WlSurface::from_id(&conn, parent_id)
                .map_err(|error| anyhow::anyhow!("failed to wrap parent wl_surface: {error:?}"))?;

            let child_surface = compositor.create_surface(&qh, ());
            let empty_region = compositor.create_region(&qh, ());
            child_surface.set_input_region(Some(&empty_region));
            empty_region.destroy();

            let subsurface = subcompositor.get_subsurface(&child_surface, &parent_surface, &qh, ());
            subsurface.place_above(&parent_surface);
            subsurface.set_desync();

            let buffer_scale = scale_factor.ceil().max(1.0) as i32;
            let mut session = Self {
                conn,
                _queue: queue,
                qh,
                shm,
                parent_surface,
                child_surface,
                subsurface,
                buffer_scale,
                buffers: Vec::new(),
                closed: false,
            };
            session.attach_new_buffer(visual)?;
            session.subsurface.set_position(local_x, local_y);
            session.parent_surface.commit();
            let _ = session.conn.flush();
            Ok(session)
        }

        fn attach_new_buffer(&mut self, visual: &DragGhostVisualSpec) -> anyhow::Result<()> {
            if self.closed {
                return Ok(());
            }
            let buf = ShmBuffer::allocate(&self.shm, &self.qh, visual, self.buffer_scale)?;
            let log_w = visual.logical_width.round().max(1.0) as i32;
            let log_h = visual.logical_height.round().max(1.0) as i32;
            let px_w = log_w.saturating_mul(self.buffer_scale);
            let px_h = log_h.saturating_mul(self.buffer_scale);

            self.child_surface.attach(Some(&buf.buffer), 0, 0);
            if self.child_surface.version() >= 3 {
                self.child_surface.set_buffer_scale(self.buffer_scale);
            }
            if self.child_surface.version() >= 4 {
                self.child_surface.damage_buffer(0, 0, px_w, px_h);
            } else {
                self.child_surface.damage(0, 0, log_w, log_h);
            }
            self.child_surface.commit();
            self.buffers.push(buf);
            Ok(())
        }

        pub(super) fn update_visual(&mut self, visual: &DragGhostVisualSpec) {
            if self.closed {
                return;
            }
            if let Err(error) = self.attach_new_buffer(visual) {
                log::debug!("failed to update Wayland drag ghost visual: {error:?}");
                return;
            }
            let _ = self.conn.flush();
        }

        pub(super) fn set_position(&self, local_x: i32, local_y: i32) {
            if self.closed {
                return;
            }
            self.subsurface.set_position(local_x, local_y);
            self.parent_surface.commit();
            let _ = self.conn.flush();
        }

        pub(super) fn close(&mut self) {
            if self.closed {
                return;
            }
            self.closed = true;
            self.subsurface.destroy();
            self.child_surface.destroy();
            for mut buf in self.buffers.drain(..) {
                buf.destroy();
            }
            self.parent_surface.commit();
            let _ = self.conn.flush();
        }
    }

    impl Drop for WaylandGhostSession {
        fn drop(&mut self) {
            self.close();
        }
    }
}

#[cfg(target_os = "linux")]
fn x11_window_and_connection(window: &gpui_kit::Window) -> Option<(usize, u32)> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

    let raw_window = HasWindowHandle::window_handle(window).ok()?.as_raw();
    let RawWindowHandle::Xcb(win) = raw_window else {
        return None;
    };
    let raw_display = HasDisplayHandle::display_handle(window).ok()?.as_raw();
    let RawDisplayHandle::Xcb(disp) = raw_display else {
        return None;
    };
    let conn = disp.connection?.as_ptr() as usize;
    if conn == 0 {
        return None;
    }
    Some((conn, win.window.get()))
}

#[cfg(target_os = "linux")]
fn wayland_surface_and_display(window: &gpui_kit::Window) -> Option<(usize, usize)> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

    let raw_window = HasWindowHandle::window_handle(window).ok()?.as_raw();
    let RawWindowHandle::Wayland(win) = raw_window else {
        return None;
    };
    let raw_display = HasDisplayHandle::display_handle(window).ok()?.as_raw();
    let RawDisplayHandle::Wayland(disp) = raw_display else {
        return None;
    };
    let surface = win.surface.as_ptr() as usize;
    let display = disp.display.as_ptr() as usize;
    if surface == 0 || display == 0 {
        return None;
    }
    Some((surface, display))
}

#[cfg(target_os = "linux")]
enum LinuxDragGhostKind {
    X11 { conn: usize, window: u32 },
    Wayland(std::sync::Mutex<Option<linux_wayland::WaylandGhostSession>>),
}

#[cfg(target_os = "linux")]
impl core::fmt::Debug for LinuxDragGhostKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::X11 { conn, window } => f
                .debug_struct("X11")
                .field("conn", conn)
                .field("window", window)
                .finish(),
            Self::Wayland(_) => f.debug_tuple("Wayland").finish(),
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct DragGhostNativeInner {
    kind: LinuxDragGhostKind,
    alive: std::sync::atomic::AtomicBool,
    move_scheduled: std::sync::atomic::AtomicBool,
    target_x: std::sync::atomic::AtomicI32,
    target_y: std::sync::atomic::AtomicI32,
}

#[cfg(target_os = "linux")]
impl Drop for DragGhostNativeInner {
    fn drop(&mut self) {
        if let LinuxDragGhostKind::Wayland(session) = &self.kind
            && let Ok(mut guard) = session.lock()
            && let Some(mut session) = guard.take()
        {
            session.close();
        }
    }
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
pub struct DragGhostNativeHandle {
    inner: std::sync::Arc<DragGhostNativeInner>,
}

#[cfg(target_os = "linux")]
impl DragGhostNativeHandle {
    pub fn invalidate(&self) {
        self.inner
            .alive
            .store(false, std::sync::atomic::Ordering::Release);
        if let LinuxDragGhostKind::Wayland(session) = &self.inner.kind
            && let Ok(mut guard) = session.lock()
            && let Some(mut session) = guard.take()
        {
            session.close();
        }
    }

    pub fn raw_hwnd(&self) -> Option<isize> {
        None
    }

    #[cfg(test)]
    pub fn is_alive(&self) -> bool {
        self.inner.alive.load(std::sync::atomic::Ordering::Acquire)
    }

    #[cfg(test)]
    fn dummy_for_test() -> Self {
        Self {
            inner: std::sync::Arc::new(DragGhostNativeInner {
                kind: LinuxDragGhostKind::X11 { conn: 0, window: 0 },
                alive: std::sync::atomic::AtomicBool::new(true),
                move_scheduled: std::sync::atomic::AtomicBool::new(false),
                target_x: std::sync::atomic::AtomicI32::new(0),
                target_y: std::sync::atomic::AtomicI32::new(0),
            }),
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragGhostNativeHandle;

#[cfg(not(any(windows, target_os = "linux")))]
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

pub fn uses_native_surface_drag_ghost(source_window: &gpui_kit::Window) -> bool {
    #[cfg(target_os = "linux")]
    {
        wayland_surface_and_display(source_window).is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = source_window;
        false
    }
}

pub fn supports_floating_drag_ghost(source_window: &gpui_kit::Window) -> bool {
    #[cfg(windows)]
    {
        window_hwnd(source_window).is_some()
    }
    #[cfg(target_os = "linux")]
    {
        (x11_window_and_connection(source_window).is_some() && linux_xcb::api().is_some())
            || wayland_surface_and_display(source_window).is_some()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = source_window;
        false
    }
}

#[cfg(target_os = "linux")]
pub fn open_native_surface_drag_ghost(
    source_window: &gpui_kit::Window,
    local_origin: gpui_kit::Point<gpui_kit::Pixels>,
    visual: &DragGhostVisualSpec,
) -> anyhow::Result<DragGhostNativeHandle> {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32},
    };

    let (surface_ptr, display_ptr) = wayland_surface_and_display(source_window)
        .ok_or_else(|| anyhow::anyhow!("GPUI did not expose a Wayland surface handle"))?;
    let local_x = f32::from(local_origin.x).round() as i32;
    let local_y = f32::from(local_origin.y).round() as i32;
    let session = linux_wayland::WaylandGhostSession::open(
        surface_ptr,
        display_ptr,
        local_x,
        local_y,
        source_window.scale_factor(),
        visual,
    )?;
    Ok(DragGhostNativeHandle {
        inner: Arc::new(DragGhostNativeInner {
            kind: LinuxDragGhostKind::Wayland(Mutex::new(Some(session))),
            alive: AtomicBool::new(true),
            move_scheduled: AtomicBool::new(false),
            target_x: AtomicI32::new(local_x),
            target_y: AtomicI32::new(local_y),
        }),
    })
}

#[cfg(not(target_os = "linux"))]
pub fn open_native_surface_drag_ghost(
    _source_window: &gpui_kit::Window,
    _local_origin: gpui_kit::Point<gpui_kit::Pixels>,
    _visual: &DragGhostVisualSpec,
) -> anyhow::Result<DragGhostNativeHandle> {
    anyhow::bail!("native surface drag ghost unsupported on this platform")
}

#[cfg(target_os = "linux")]
pub fn update_native_drag_ghost_visual(
    handle: &DragGhostNativeHandle,
    visual: &DragGhostVisualSpec,
) {
    if !handle
        .inner
        .alive
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return;
    }
    if let LinuxDragGhostKind::Wayland(session) = &handle.inner.kind
        && let Ok(mut guard) = session.lock()
        && let Some(session) = guard.as_mut()
    {
        session.update_visual(visual);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn update_native_drag_ghost_visual(
    _handle: &DragGhostNativeHandle,
    _visual: &DragGhostVisualSpec,
) {
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
        WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        WS_EX_TRANSPARENT,
    };

    let raw_hwnd = window_hwnd(window)
        .ok_or_else(|| anyhow::anyhow!("GPUI did not expose a Win32 drag ghost window handle"))?;
    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    let current_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let uses_direct_composition = (current_style & WS_EX_NOREDIRECTIONBITMAP.0 as isize) != 0;
    let style = (current_style & !(WS_EX_APPWINDOW.0 as isize))
        | WS_EX_TOOLWINDOW.0 as isize
        | WS_EX_NOACTIVATE.0 as isize
        | WS_EX_TRANSPARENT.0 as isize
        | WS_EX_TOPMOST.0 as isize;
    unsafe {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
    }
    configure_transparent_drag_ghost_dwm(hwnd, uses_direct_composition);

    let mut rect = RECT::default();
    let (phys_w, phys_h) = if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        let phys_w = rect.right.saturating_sub(rect.left);
        let phys_h = rect.bottom.saturating_sub(rect.top);
        if !uses_direct_composition && phys_w > 0 && phys_h > 0 {
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
            uses_direct_composition,
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

#[cfg(target_os = "linux")]
pub fn configure_drag_ghost_window(
    window: &gpui_kit::Window,
    _grab_offset: gpui_kit::Point<gpui_kit::Pixels>,
    _logical_height: f32,
    _corner_radius: f32,
) -> anyhow::Result<DragGhostNativeHandle> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32},
    };

    let (conn, window_id) = x11_window_and_connection(window)
        .ok_or_else(|| anyhow::anyhow!("GPUI did not expose an X11 drag ghost window handle"))?;
    let api = linux_xcb::api()
        .ok_or_else(|| anyhow::anyhow!("libxcb is unavailable for floating drag ghost"))?;
    if let Some(shape_rectangles) = api.shape_rectangles {
        const XCB_SHAPE_SO_SET: u8 = 0;
        const XCB_SHAPE_SK_INPUT: u8 = 2;
        const XCB_CLIP_ORDERING_UNSORTED: u8 = 0;
        unsafe {
            let _ = shape_rectangles(
                conn as *mut core::ffi::c_void,
                XCB_SHAPE_SO_SET,
                XCB_SHAPE_SK_INPUT,
                XCB_CLIP_ORDERING_UNSORTED,
                window_id,
                0,
                0,
                0,
                core::ptr::null(),
            );
            let _ = (api.flush)(conn as *mut core::ffi::c_void);
        }
    }

    Ok(DragGhostNativeHandle {
        inner: Arc::new(DragGhostNativeInner {
            kind: LinuxDragGhostKind::X11 {
                conn,
                window: window_id,
            },
            alive: AtomicBool::new(true),
            move_scheduled: AtomicBool::new(false),
            target_x: AtomicI32::new(0),
            target_y: AtomicI32::new(0),
        }),
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
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
    _target_origin: gpui_kit::Point<gpui_kit::Pixels>,
    _local_origin: gpui_kit::Point<gpui_kit::Pixels>,
    _scale_factor: f32,
    grab_offset: gpui_kit::Point<gpui_kit::Pixels>,
    logical_height: f32,
    corner_radius: f32,
    cx: &gpui_kit::App,
) {
    use std::sync::{Arc, atomic::Ordering};
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetCursorPos, GetWindowRect, HWND_TOPMOST, IsWindow, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
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
            if !inner.uses_direct_composition
                && (phys_w, phys_h) != (prev_w, prev_h)
                && phys_w > 0
                && phys_h > 0
            {
                let diameter = scaled_region_diameter(phys_h, logical_height, corner_radius);
                apply_rounded_window_region(hwnd, phys_w, phys_h, diameter, "tab drag ghost");
                inner.last_physical_width.store(phys_w, Ordering::Relaxed);
                inner.last_physical_height.store(phys_h, Ordering::Relaxed);
            }

            let mut client_rect = RECT::default();
            let (client_h, left_inset, top_inset) =
                if unsafe { GetClientRect(hwnd, &mut client_rect) }.is_ok() {
                    let mut client_origin = POINT {
                        x: client_rect.left,
                        y: client_rect.top,
                    };
                    let client_h = client_rect.bottom.saturating_sub(client_rect.top);
                    if client_h > 0 && unsafe { ClientToScreen(hwnd, &mut client_origin) }.as_bool()
                    {
                        (
                            client_h,
                            client_origin.x - rect.left,
                            client_origin.y - rect.top,
                        )
                    } else {
                        (phys_h, 0, 0)
                    }
                } else {
                    (phys_h, 0, 0)
                };

            let scale = if logical_height > 0.0 && client_h > 0 {
                (client_h as f32 / logical_height).max(0.5)
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
            let x = cursor.x - (offset_x * scale).round() as i32 - left_inset;
            let y = cursor.y - (offset_y * scale).round() as i32 - top_inset;

            if !inner.alive.load(Ordering::Acquire) || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                return;
            }
            let frame_changed = inner.needs_frame_change.swap(false, Ordering::AcqRel);
            let mut flags = SWP_NOACTIVATE | SWP_NOSIZE | SWP_SHOWWINDOW;
            if frame_changed {
                flags |= SWP_FRAMECHANGED;
            }
            if let Err(error) = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, 0, 0, flags) }
            {
                log::debug!("failed to move drag ghost window: {error:?}");
            } else if frame_changed && inner.uses_direct_composition {
                disable_window_composition_accent(hwnd);
            }
        })
        .detach();
}

#[cfg(target_os = "linux")]
pub fn schedule_drag_ghost_move(
    handle: &DragGhostNativeHandle,
    target_origin: gpui_kit::Point<gpui_kit::Pixels>,
    local_origin: gpui_kit::Point<gpui_kit::Pixels>,
    scale_factor: f32,
    _grab_offset: gpui_kit::Point<gpui_kit::Pixels>,
    _logical_height: f32,
    _corner_radius: f32,
    cx: &gpui_kit::App,
) {
    use std::sync::{Arc, atomic::Ordering};

    if !handle.inner.alive.load(Ordering::Acquire) {
        return;
    }
    let (x, y) = match &handle.inner.kind {
        LinuxDragGhostKind::X11 { .. } => {
            let scale = scale_factor.max(0.5);
            (
                (f32::from(target_origin.x) * scale).round() as i32,
                (f32::from(target_origin.y) * scale).round() as i32,
            )
        }
        LinuxDragGhostKind::Wayland(_) => (
            f32::from(local_origin.x).round() as i32,
            f32::from(local_origin.y).round() as i32,
        ),
    };
    handle.inner.target_x.store(x, Ordering::Relaxed);
    handle.inner.target_y.store(y, Ordering::Relaxed);
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
            let x = inner.target_x.load(Ordering::Relaxed);
            let y = inner.target_y.load(Ordering::Relaxed);
            match &inner.kind {
                LinuxDragGhostKind::X11 { conn, window } => {
                    let Some(api) = linux_xcb::api() else {
                        return;
                    };
                    const XCB_CONFIG_WINDOW_X: u16 = 1;
                    const XCB_CONFIG_WINDOW_Y: u16 = 2;
                    const XCB_CONFIG_WINDOW_STACK_MODE: u16 = 64;
                    const XCB_STACK_MODE_ABOVE: u32 = 0;

                    let values: [u32; 3] = [x as u32, y as u32, XCB_STACK_MODE_ABOVE];
                    unsafe {
                        let _ = (api.configure_window)(
                            *conn as *mut core::ffi::c_void,
                            *window,
                            XCB_CONFIG_WINDOW_X
                                | XCB_CONFIG_WINDOW_Y
                                | XCB_CONFIG_WINDOW_STACK_MODE,
                            values.as_ptr().cast(),
                        );
                        let _ = (api.flush)(*conn as *mut core::ffi::c_void);
                    }
                }
                LinuxDragGhostKind::Wayland(session) => {
                    if let Ok(guard) = session.lock()
                        && let Some(session) = guard.as_ref()
                    {
                        session.set_position(x, y);
                    }
                }
            }
        })
        .detach();
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn schedule_drag_ghost_move(
    _handle: &DragGhostNativeHandle,
    _target_origin: gpui_kit::Point<gpui_kit::Pixels>,
    _local_origin: gpui_kit::Point<gpui_kit::Pixels>,
    _scale_factor: f32,
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

#[cfg(all(test, any(windows, target_os = "linux")))]
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

    #[test]
    fn drag_ghost_native_handle_invalidate_marks_dead() {
        let handle = DragGhostNativeHandle::dummy_for_test();
        assert!(handle.is_alive());
        handle.invalidate();
        assert!(!handle.is_alive());
    }
}
