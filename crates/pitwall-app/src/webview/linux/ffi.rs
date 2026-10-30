//! The few GLib, GTK 3, GDK, cairo, Xlib and WebKitGTK functions the
//! backend calls, loaded at run time (`dlopen`): Pitwall doesn't link
//! WebKitGTK, so it starts on machines without it and the preview falls
//! back to the browser there. Hand-written signatures (C headers of
//! webkit2gtk 4.0/4.1, GTK 3.24); every pointer is an opaque `P`.

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::sync::OnceLock;

use libloading::Library;

/// An opaque C pointer (GObject, GtkWidget, GdkEvent, Display…).
pub type P = *mut c_void;
/// `GCallback`: a signal handler, cast from its real signature.
pub type GCallback = Option<unsafe extern "C" fn()>;
pub type GDestroyNotify = Option<unsafe extern "C" fn(P, P)>;
pub type GAsyncReadyCallback = Option<unsafe extern "C" fn(P, P, P)>;

/// GLib's `GError`.
#[repr(C)]
pub struct GError {
    pub domain: u32,
    pub code: c_int,
    pub message: *const c_char,
}

/// WebKitGTK for GTK 3: 4.1 (libsoup 3, current distributions), else 4.0
/// (libsoup 2, older ones). Not webkitgtk-6.0: GTK 4 can't hand its window
/// to another X11 window.
const WEBKIT: &[&str] = &["libwebkit2gtk-4.1.so.0", "libwebkit2gtk-4.0.so.37"];

struct Libs {
    webkit: Library,
    gtk: Library,
    gdk: Library,
    gobject: Library,
    glib: Library,
    cairo: Library,
    x11: Library,
}

fn open(names: &[&str]) -> Result<Library, String> {
    let mut last = String::new();
    for name in names {
        // SAFETY: these are the system's GTK stack; their initializers only
        // register types.
        match unsafe { Library::new(name) } {
            Ok(lib) => return Ok(lib),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// A function from `lib`, copied out of its symbol (the library lives as
/// long as [`Api`], which is never dropped).
unsafe fn sym<T: Copy>(lib: &Library, name: &str) -> Result<T, String> {
    match unsafe { lib.get::<T>(name.as_bytes()) } {
        Ok(s) => Ok(*s),
        Err(_) => Err(format!("WebKitGTK is missing {name} (too old?)")),
    }
}

macro_rules! api {
    ($($lib:ident { $($name:ident: $ty:ty;)* })*) => {
        /// The loaded functions (fields named as in C).
        #[allow(clippy::type_complexity, non_snake_case)]
        pub struct Api {
            $($(pub $name: $ty,)*)*
            /// WebKitGTK 2.40+; older ones tell the main frame by its URI.
            pub webkit_response_policy_decision_is_main_frame_main_resource:
                Option<unsafe extern "C" fn(P) -> c_int>,
            _libs: Libs,
        }

        impl Api {
            fn load() -> Result<Api, String> {
                let webkit = open(WEBKIT).map_err(|e| {
                    format!("WebKitGTK isn't installed (libwebkit2gtk-4.1): {e}")
                })?;
                let libs = Libs {
                    webkit,
                    gtk: open(&["libgtk-3.so.0"])?,
                    gdk: open(&["libgdk-3.so.0"])?,
                    gobject: open(&["libgobject-2.0.so.0"])?,
                    glib: open(&["libglib-2.0.so.0"])?,
                    cairo: open(&["libcairo.so.2"])?,
                    x11: open(&["libX11.so.6"])?,
                };
                // SAFETY: each type below is the C function's signature.
                unsafe {
                    Ok(Api {
                        $($($name: sym(&libs.$lib, stringify!($name))?,)*)*
                        webkit_response_policy_decision_is_main_frame_main_resource: sym(
                            &libs.webkit,
                            "webkit_response_policy_decision_is_main_frame_main_resource",
                        )
                        .ok(),
                        _libs: libs,
                    })
                }
            }
        }
    };
}

api! {
    glib {
        g_main_context_iteration: unsafe extern "C" fn(P, c_int) -> c_int;
        g_error_free: unsafe extern "C" fn(*mut GError);
    }
    gobject {
        g_signal_connect_data: unsafe extern "C" fn(P, *const c_char, GCallback, P, GDestroyNotify, c_uint) -> c_ulong;
        g_object_ref: unsafe extern "C" fn(P) -> P;
        g_object_ref_sink: unsafe extern "C" fn(P) -> P;
        g_object_unref: unsafe extern "C" fn(P);
    }
    gtk {
        gtk_disable_setlocale: unsafe extern "C" fn();
        gtk_init_check: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) -> c_int;
        gtk_window_new: unsafe extern "C" fn(c_int) -> P;
        gtk_window_set_decorated: unsafe extern "C" fn(P, c_int);
        gtk_window_set_skip_taskbar_hint: unsafe extern "C" fn(P, c_int);
        gtk_window_set_skip_pager_hint: unsafe extern "C" fn(P, c_int);
        gtk_window_move: unsafe extern "C" fn(P, c_int, c_int);
        gtk_window_resize: unsafe extern "C" fn(P, c_int, c_int);
        gtk_container_add: unsafe extern "C" fn(P, P);
        gtk_widget_realize: unsafe extern "C" fn(P);
        gtk_widget_show_all: unsafe extern "C" fn(P);
        gtk_widget_hide: unsafe extern "C" fn(P);
        gtk_widget_destroy: unsafe extern "C" fn(P);
        gtk_widget_grab_focus: unsafe extern "C" fn(P);
        gtk_widget_get_window: unsafe extern "C" fn(P) -> P;
        gtk_widget_get_scale_factor: unsafe extern "C" fn(P) -> c_int;
    }
    gdk {
        gdk_set_allowed_backends: unsafe extern "C" fn(*const c_char);
        gdk_window_get_display: unsafe extern "C" fn(P) -> P;
        gdk_x11_window_get_xid: unsafe extern "C" fn(P) -> c_ulong;
        gdk_x11_display_get_xdisplay: unsafe extern "C" fn(P) -> P;
        gdk_x11_display_error_trap_push: unsafe extern "C" fn(P);
        gdk_x11_display_error_trap_pop_ignored: unsafe extern "C" fn(P);
        gdk_event_get_keyval: unsafe extern "C" fn(P, *mut c_uint) -> c_int;
        gdk_event_get_state: unsafe extern "C" fn(P, *mut c_uint) -> c_int;
    }
    x11 {
        XReparentWindow: unsafe extern "C" fn(P, c_ulong, c_ulong, c_int, c_int) -> c_int;
        XSetInputFocus: unsafe extern "C" fn(P, c_ulong, c_int, c_ulong) -> c_int;
        XFlush: unsafe extern "C" fn(P) -> c_int;
    }
    cairo {
        cairo_surface_flush: unsafe extern "C" fn(P);
        cairo_surface_destroy: unsafe extern "C" fn(P);
        cairo_image_surface_get_data: unsafe extern "C" fn(P) -> *mut u8;
        cairo_image_surface_get_format: unsafe extern "C" fn(P) -> c_int;
        cairo_image_surface_get_width: unsafe extern "C" fn(P) -> c_int;
        cairo_image_surface_get_height: unsafe extern "C" fn(P) -> c_int;
        cairo_image_surface_get_stride: unsafe extern "C" fn(P) -> c_int;
    }
    webkit {
        webkit_website_data_manager_new: unsafe extern "C" fn(*const c_char, ...) -> P;
        webkit_web_context_new_with_website_data_manager: unsafe extern "C" fn(P) -> P;
        webkit_web_context_get_cookie_manager: unsafe extern "C" fn(P) -> P;
        webkit_cookie_manager_set_persistent_storage: unsafe extern "C" fn(P, *const c_char, c_int);
        webkit_web_view_new_with_context: unsafe extern "C" fn(P) -> P;
        webkit_web_view_load_uri: unsafe extern "C" fn(P, *const c_char);
        webkit_web_view_reload: unsafe extern "C" fn(P);
        webkit_web_view_go_back: unsafe extern "C" fn(P);
        webkit_web_view_go_forward: unsafe extern "C" fn(P);
        webkit_web_view_can_go_back: unsafe extern "C" fn(P) -> c_int;
        webkit_web_view_can_go_forward: unsafe extern "C" fn(P) -> c_int;
        webkit_web_view_get_uri: unsafe extern "C" fn(P) -> *const c_char;
        webkit_web_view_get_title: unsafe extern "C" fn(P) -> *const c_char;
        webkit_web_view_get_back_forward_list: unsafe extern "C" fn(P) -> P;
        webkit_web_view_set_zoom_level: unsafe extern "C" fn(P, f64);
        webkit_web_view_get_settings: unsafe extern "C" fn(P) -> P;
        webkit_settings_set_enable_developer_extras: unsafe extern "C" fn(P, c_int);
        webkit_web_view_get_inspector: unsafe extern "C" fn(P) -> P;
        webkit_web_inspector_show: unsafe extern "C" fn(P);
        webkit_web_view_get_snapshot: unsafe extern "C" fn(P, c_int, c_int, P, GAsyncReadyCallback, P);
        webkit_web_view_get_snapshot_finish: unsafe extern "C" fn(P, P, *mut *mut GError) -> P;
        webkit_policy_decision_ignore: unsafe extern "C" fn(P);
        webkit_navigation_policy_decision_get_navigation_action: unsafe extern "C" fn(P) -> P;
        webkit_navigation_action_get_request: unsafe extern "C" fn(P) -> P;
        webkit_navigation_action_get_navigation_type: unsafe extern "C" fn(P) -> c_int;
        webkit_navigation_action_is_user_gesture: unsafe extern "C" fn(P) -> c_int;
        webkit_uri_request_get_uri: unsafe extern "C" fn(P) -> *const c_char;
        webkit_response_policy_decision_get_response: unsafe extern "C" fn(P) -> P;
        webkit_uri_response_get_uri: unsafe extern "C" fn(P) -> *const c_char;
        webkit_network_error_quark: unsafe extern "C" fn() -> u32;
        webkit_policy_error_quark: unsafe extern "C" fn() -> u32;
    }
}

/// The library, loaded once (on the first preview, not at startup).
pub fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(Api::load).as_ref().map_err(Clone::clone)
}
