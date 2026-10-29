//! Dynload bindings for the libmpv client + software render APIs.
//!
//! Prefer system `libmpv.so.2` / `libmpv.dylib` so the release binary stays small
//! (no bundled libmpv). Compile-time linking is intentionally avoided.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::ptr;
use std::sync::Arc;

use libloading::Library;

pub type mpv_handle = c_void;
pub type mpv_render_context = c_void;

pub type mpv_format = u32;
pub const MPV_FORMAT_NONE: mpv_format = 0;
pub const MPV_FORMAT_STRING: mpv_format = 1;
pub const MPV_FORMAT_FLAG: mpv_format = 3;
pub const MPV_FORMAT_INT64: mpv_format = 4;
pub const MPV_FORMAT_DOUBLE: mpv_format = 5;
pub const MPV_FORMAT_NODE: mpv_format = 6;
pub const MPV_FORMAT_NODE_ARRAY: mpv_format = 7;
pub const MPV_FORMAT_NODE_MAP: mpv_format = 8;
pub const MPV_FORMAT_BYTE_ARRAY: mpv_format = 9;

pub type mpv_event_id = i32;
pub const MPV_EVENT_NONE: mpv_event_id = 0;
pub const MPV_EVENT_SHUTDOWN: mpv_event_id = 1;
pub const MPV_EVENT_FILE_LOADED: mpv_event_id = 8;
pub const MPV_EVENT_START_FILE: mpv_event_id = 6;
pub const MPV_EVENT_END_FILE: mpv_event_id = 7;
pub const MPV_EVENT_CLIENT_MESSAGE: mpv_event_id = 16;
pub const MPV_EVENT_VIDEO_RECONFIG: mpv_event_id = 17;
pub const MPV_EVENT_PLAYBACK_RESTART: mpv_event_id = 21;
pub const MPV_EVENT_PROPERTY_CHANGE: mpv_event_id = 22;

pub type mpv_end_file_reason = i32;
pub const MPV_END_FILE_REASON_EOF: mpv_end_file_reason = 0;
pub const MPV_END_FILE_REASON_STOP: mpv_end_file_reason = 2;
pub const MPV_END_FILE_REASON_ERROR: mpv_end_file_reason = 4;

pub type mpv_render_param_type = u32;
pub const MPV_RENDER_PARAM_INVALID: mpv_render_param_type = 0;
pub const MPV_RENDER_PARAM_API_TYPE: mpv_render_param_type = 1;
pub const MPV_RENDER_PARAM_SW_SIZE: mpv_render_param_type = 17;
pub const MPV_RENDER_PARAM_SW_FORMAT: mpv_render_param_type = 18;
pub const MPV_RENDER_PARAM_SW_STRIDE: mpv_render_param_type = 19;
pub const MPV_RENDER_PARAM_SW_POINTER: mpv_render_param_type = 20;

#[repr(C)]
pub struct mpv_node {
    pub u: mpv_node_u,
    pub format: mpv_format,
}

#[repr(C)]
pub union mpv_node_u {
    pub string: *mut c_char,
    pub flag: c_int,
    pub int64: i64,
    pub double_: c_double,
    pub list: *mut mpv_node_list,
    pub ba: *mut mpv_byte_array,
}

#[repr(C)]
pub struct mpv_node_list {
    pub num: c_int,
    pub values: *mut mpv_node,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
pub struct mpv_byte_array {
    pub data: *mut c_void,
    pub size: usize,
}

#[repr(C)]
pub struct mpv_event {
    pub event_id: mpv_event_id,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_end_file {
    pub reason: mpv_end_file_reason,
    pub error: c_int,
    pub playlist_entry_id: i64,
}

#[repr(C)]
pub struct mpv_event_client_message {
    pub num_args: c_int,
    pub args: *mut *const c_char,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: mpv_format,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_render_param {
    pub type_: mpv_render_param_type,
    pub data: *mut c_void,
}

type FnCreate = unsafe extern "C" fn() -> *mut mpv_handle;
type FnInitialize = unsafe extern "C" fn(*mut mpv_handle) -> c_int;
type FnTerminateDestroy = unsafe extern "C" fn(*mut mpv_handle);
type FnCommand = unsafe extern "C" fn(*mut mpv_handle, *const *const c_char) -> c_int;
type FnCommandString = unsafe extern "C" fn(*mut mpv_handle, *const c_char) -> c_int;
type FnSetPropertyString = unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int;
type FnSetProperty = unsafe extern "C" fn(*mut mpv_handle, *const c_char, mpv_format, *mut c_void) -> c_int;
type FnGetProperty = unsafe extern "C" fn(*mut mpv_handle, *const c_char, mpv_format, *mut c_void) -> c_int;
type FnGetPropertyString = unsafe extern "C" fn(*mut mpv_handle, *const c_char) -> *mut c_char;
type FnObserveProperty = unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, mpv_format) -> c_int;
type FnWaitEvent = unsafe extern "C" fn(*mut mpv_handle, c_double) -> *mut mpv_event;
type FnWakeup = unsafe extern "C" fn(*mut mpv_handle);
type FnFree = unsafe extern "C" fn(*mut c_void);
type FnFreeNodeContents = unsafe extern "C" fn(*mut mpv_node);
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type FnSetOptionString = unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int;
type FnRenderCreate = unsafe extern "C" fn(*mut *mut mpv_render_context, *mut mpv_handle, *mut mpv_render_param) -> c_int;
type FnRenderFree = unsafe extern "C" fn(*mut mpv_render_context);
type FnRender = unsafe extern "C" fn(*mut mpv_render_context, *mut mpv_render_param) -> c_int;
type FnRenderUpdate = unsafe extern "C" fn(*mut mpv_render_context) -> u64;
type FnRenderSetUpdateCallback = unsafe extern "C" fn(*mut mpv_render_context, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void);

pub struct MpvLib {
    _lib: Library,
    pub create: FnCreate,
    pub initialize: FnInitialize,
    pub terminate_destroy: FnTerminateDestroy,
    pub command: FnCommand,
    pub command_string: FnCommandString,
    pub set_property_string: FnSetPropertyString,
    pub set_property: FnSetProperty,
    pub get_property: FnGetProperty,
    pub get_property_string: FnGetPropertyString,
    pub observe_property: FnObserveProperty,
    pub wait_event: FnWaitEvent,
    pub wakeup: FnWakeup,
    pub free: FnFree,
    pub free_node_contents: FnFreeNodeContents,
    pub error_string: FnErrorString,
    pub set_option_string: FnSetOptionString,
    pub render_create: FnRenderCreate,
    pub render_free: FnRenderFree,
    pub render: FnRender,
    pub render_update: FnRenderUpdate,
    pub render_set_update_callback: FnRenderSetUpdateCallback,
}

fn load_sym<T: Copy>(lib: &Library, name: &[u8]) -> Result<T, String> {
    unsafe {
        lib.get::<T>(name)
            .map(|s| *s)
            .map_err(|e| format!("libmpv missing symbol {}: {e}", String::from_utf8_lossy(name)))
    }
}

impl MpvLib {
    pub fn load() -> Result<Arc<Self>, String> {
        let candidates: &[&str] = if cfg!(target_os = "macos") {
            &[
                "libmpv.2.dylib",
                "libmpv.dylib",
                "/opt/homebrew/lib/libmpv.dylib",
                "/usr/local/lib/libmpv.dylib",
            ]
        } else {
            &[
                "libmpv.so.2",
                "libmpv.so",
                "/usr/lib/x86_64-linux-gnu/libmpv.so.2",
                "/usr/lib/libmpv.so.2",
            ]
        };
        let mut last = String::from("libmpv not found");
        for name in candidates {
            match unsafe { Library::new(name) } {
                Ok(lib) => match Self::from_library(lib) {
                    Ok(v) => return Ok(Arc::new(v)),
                    Err(e) => last = e,
                },
                Err(e) => last = format!("dlopen({name}): {e}"),
            }
        }
        Err(format!(
            "Cannot load system libmpv ({last}). Install libmpv (Debian/Ubuntu: libmpv2 / libmpv-dev; Fedora: mpv-libs; macOS: brew install mpv)."
        ))
    }

    fn from_library(lib: Library) -> Result<Self, String> {
        Ok(Self {
            create: load_sym(&lib, b"mpv_create\0")?,
            initialize: load_sym(&lib, b"mpv_initialize\0")?,
            terminate_destroy: load_sym(&lib, b"mpv_terminate_destroy\0")?,
            command: load_sym(&lib, b"mpv_command\0")?,
            command_string: load_sym(&lib, b"mpv_command_string\0")?,
            set_property_string: load_sym(&lib, b"mpv_set_property_string\0")?,
            set_property: load_sym(&lib, b"mpv_set_property\0")?,
            get_property: load_sym(&lib, b"mpv_get_property\0")?,
            get_property_string: load_sym(&lib, b"mpv_get_property_string\0")?,
            observe_property: load_sym(&lib, b"mpv_observe_property\0")?,
            wait_event: load_sym(&lib, b"mpv_wait_event\0")?,
            wakeup: load_sym(&lib, b"mpv_wakeup\0")?,
            free: load_sym(&lib, b"mpv_free\0")?,
            free_node_contents: load_sym(&lib, b"mpv_free_node_contents\0")?,
            error_string: load_sym(&lib, b"mpv_error_string\0")?,
            set_option_string: load_sym(&lib, b"mpv_set_option_string\0")?,
            render_create: load_sym(&lib, b"mpv_render_context_create\0")?,
            render_free: load_sym(&lib, b"mpv_render_context_free\0")?,
            render: load_sym(&lib, b"mpv_render_context_render\0")?,
            render_update: load_sym(&lib, b"mpv_render_context_update\0")?,
            render_set_update_callback: load_sym(&lib, b"mpv_render_context_set_update_callback\0")?,
            _lib: lib,
        })
    }

    pub fn err_str(&self, code: c_int) -> String {
        if code >= 0 {
            return "ok".into();
        }
        unsafe {
            let p = (self.error_string)(code);
            if p.is_null() {
                format!("mpv error {code}")
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }
}

pub struct Handle {
    pub lib: Arc<MpvLib>,
    pub ptr: *mut mpv_handle,
    pub render: *mut mpv_render_context,
}

// libmpv handles are documented as usable from one thread; we keep them on the video thread.
unsafe impl Send for Handle {}

impl Handle {
    pub fn create(lib: Arc<MpvLib>) -> Result<Self, String> {
        let ptr = unsafe { (lib.create)() };
        if ptr.is_null() {
            return Err("mpv_create returned null".into());
        }
        Ok(Self {
            lib,
            ptr,
            render: ptr::null_mut(),
        })
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), String> {
        let n = CString::new(name).unwrap();
        let v = CString::new(value).unwrap();
        let rc = unsafe { (self.lib.set_option_string)(self.ptr, n.as_ptr(), v.as_ptr()) };
        if rc < 0 {
            Err(format!("set_option {name}={value}: {}", self.lib.err_str(rc)))
        } else {
            Ok(())
        }
    }

    pub fn initialize(&self) -> Result<(), String> {
        let rc = unsafe { (self.lib.initialize)(self.ptr) };
        if rc < 0 {
            Err(format!("mpv_initialize: {}", self.lib.err_str(rc)))
        } else {
            Ok(())
        }
    }

    pub fn set_prop_str(&self, name: &str, value: &str) -> Result<(), String> {
        let n = CString::new(name).unwrap();
        let v = CString::new(value).unwrap();
        let rc = unsafe { (self.lib.set_property_string)(self.ptr, n.as_ptr(), v.as_ptr()) };
        if rc < 0 {
            Err(format!("set {name}: {}", self.lib.err_str(rc)))
        } else {
            Ok(())
        }
    }

    pub fn set_prop_flag(&self, name: &str, value: bool) -> Result<(), String> {
        let n = CString::new(name).unwrap();
        let mut flag: c_int = if value { 1 } else { 0 };
        let rc = unsafe {
            (self.lib.set_property)(
                self.ptr,
                n.as_ptr(),
                MPV_FORMAT_FLAG,
                &mut flag as *mut _ as *mut c_void,
            )
        };
        if rc < 0 {
            Err(format!("set {name}: {}", self.lib.err_str(rc)))
        } else {
            Ok(())
        }
    }

    pub fn set_prop_f64(&self, name: &str, value: f64) -> Result<(), String> {
        let n = CString::new(name).unwrap();
        let mut v = value;
        let rc = unsafe {
            (self.lib.set_property)(
                self.ptr,
                n.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut v as *mut _ as *mut c_void,
            )
        };
        if rc < 0 {
            Err(format!("set {name}: {}", self.lib.err_str(rc)))
        } else {
            Ok(())
        }
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let n = CString::new(name).ok()?;
        let mut v: c_double = 0.0;
        let rc = unsafe {
            (self.lib.get_property)(
                self.ptr,
                n.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut v as *mut _ as *mut c_void,
            )
        };
        if rc < 0 {
            None
        } else {
            Some(v)
        }
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let n = CString::new(name).ok()?;
        let mut v: i64 = 0;
        let rc = unsafe {
            (self.lib.get_property)(
                self.ptr,
                n.as_ptr(),
                MPV_FORMAT_INT64,
                &mut v as *mut _ as *mut c_void,
            )
        };
        if rc < 0 {
            None
        } else {
            Some(v)
        }
    }

    pub fn get_flag(&self, name: &str) -> Option<bool> {
        let n = CString::new(name).ok()?;
        let mut v: c_int = 0;
        let rc = unsafe {
            (self.lib.get_property)(
                self.ptr,
                n.as_ptr(),
                MPV_FORMAT_FLAG,
                &mut v as *mut _ as *mut c_void,
            )
        };
        if rc < 0 {
            None
        } else {
            Some(v != 0)
        }
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let c_args: Vec<CString> = args.iter().map(|s| CString::new(*s).unwrap()).collect();
        let mut ptrs: Vec<*const c_char> = c_args.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(ptr::null());
        let rc = unsafe { (self.lib.command)(self.ptr, ptrs.as_ptr()) };
        if rc < 0 {
            Err(format!("command {:?}: {}", args, self.lib.err_str(rc)))
        } else {
            Ok(())
        }
    }

    pub fn observe(&self, name: &str, format: mpv_format) -> Result<(), String> {
        let n = CString::new(name).unwrap();
        let rc = unsafe { (self.lib.observe_property)(self.ptr, 0, n.as_ptr(), format) };
        if rc < 0 {
            Err(format!("observe {name}: {}", self.lib.err_str(rc)))
        } else {
            Ok(())
        }
    }

    pub fn create_sw_render(&mut self) -> Result<(), String> {
        if !self.render.is_null() {
            return Ok(());
        }
        let api = CString::new("sw").unwrap();
        let mut params = [
            mpv_render_param {
                type_: MPV_RENDER_PARAM_API_TYPE,
                data: api.as_ptr() as *mut c_void,
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        let mut ctx: *mut mpv_render_context = ptr::null_mut();
        let rc = unsafe { (self.lib.render_create)(&mut ctx, self.ptr, params.as_mut_ptr()) };
        if rc < 0 || ctx.is_null() {
            Err(format!("mpv_render_context_create(sw): {}", self.lib.err_str(rc)))
        } else {
            self.render = ctx;
            Ok(())
        }
    }

    pub fn render_sw(&self, w: i32, h: i32, stride: usize, pixels: &mut [u8]) -> Result<(), String> {
        if self.render.is_null() {
            return Err("no render context".into());
        }
        let mut size = [w, h];
        // Documented SW formats: rgb0 / bgr0 / 0rgb / 0bgr (4 bytes; "0" = padding).
        let fmt = CString::new("rgb0").unwrap();
        let mut stride_v = stride;
        let mut params = [
            mpv_render_param {
                type_: MPV_RENDER_PARAM_SW_SIZE,
                data: size.as_mut_ptr() as *mut c_void,
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_SW_FORMAT,
                data: fmt.as_ptr() as *mut c_void,
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_SW_STRIDE,
                data: &mut stride_v as *mut _ as *mut c_void,
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_SW_POINTER,
                data: pixels.as_mut_ptr() as *mut c_void,
            },
            mpv_render_param {
                type_: MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        let rc = unsafe { (self.lib.render)(self.render, params.as_mut_ptr()) };
        if rc < 0 {
            return Err(format!("render: {}", self.lib.err_str(rc)));
        }
        // rgb0 leaves the 4th byte unspecified — force opaque for egui.
        for px in pixels.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(())
    }

    pub fn wakeup(&self) {
        unsafe { (self.lib.wakeup)(self.ptr) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.render.is_null() {
            unsafe { (self.lib.render_free)(self.render) };
            self.render = ptr::null_mut();
        }
        if !self.ptr.is_null() {
            unsafe { (self.lib.terminate_destroy)(self.ptr) };
            self.ptr = ptr::null_mut();
        }
    }
}
