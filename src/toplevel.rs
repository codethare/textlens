//! The focused toplevel's `ext_foreign_toplevel_handle_v1.identifier`, which
//! `grim -T` accepts.
//!
//! A compositor like river gives a third-party client no window geometry:
//! `river_window_manager_v1` is exclusive to the window manager (tailrace).
//! What is exported is `ext_foreign_toplevel_list_v1`, whose handles carry the
//! stable identifier, while *which* toplevel is focused only exists in
//! `zwlr_foreign_toplevel_manager_v1`. Both are bound here and matched up.
//!
//! shortcut: focus is matched to an identifier on title+app_id, so two windows
//! with the same title and app_id are ambiguous; upgrade when
//! ext-foreign-toplevel-list carries activation state.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

const ACTIVATED: u32 = 2;
const LIST: &CStr = c"ext_foreign_toplevel_list_v1";
const MANAGER: &CStr = c"zwlr_foreign_toplevel_manager_v1";

#[repr(C)]
struct WlMessage {
    name: *const c_char,
    signature: *const c_char,
    types: *const *const WlInterface,
}

#[repr(C)]
struct WlInterface {
    name: *const c_char,
    version: c_int,
    method_count: c_int,
    methods: *const WlMessage,
    event_count: c_int,
    events: *const WlMessage,
}

#[repr(C)]
struct WlArray {
    size: usize,
    alloc: usize,
    data: *const u8,
}

type Proxy = *mut c_void;

mod ffi {
    use super::{Proxy, WlInterface, c_char, c_int, c_void};

    #[link(name = "wayland-client")]
    unsafe extern "C" {
        pub fn wl_display_connect(name: *const c_char) -> Proxy;
        pub fn wl_display_roundtrip(display: Proxy) -> c_int;
        pub fn wl_proxy_add_listener(
            proxy: Proxy,
            implementation: *const *const c_void,
            data: *mut c_void,
        ) -> c_int;
        pub fn wl_proxy_marshal_flags(
            proxy: Proxy,
            opcode: u32,
            interface: *const WlInterface,
            version: u32,
            flags: u32,
            ...
        ) -> Proxy;
        pub static wl_registry_interface: WlInterface;
        pub static wl_output_interface: WlInterface;
    }
}

/// One `[name, signature, types]` table; `types` is non-null only for messages
/// taking objects, where libwayland needs the interface for demarshalling.
fn table(
    spec: &[(&'static CStr, &'static CStr, *const *const WlInterface)],
) -> (*const WlMessage, c_int) {
    let messages: Vec<WlMessage> = spec
        .iter()
        .map(|&(name, signature, types)| WlMessage {
            name: name.as_ptr(),
            signature: signature.as_ptr(),
            types,
        })
        .collect();
    let count = messages.len() as c_int;
    (Box::leak(messages.into_boxed_slice()).as_ptr(), count)
}

fn types(interfaces: &[*const WlInterface]) -> *const *const WlInterface {
    Box::leak(interfaces.to_vec().into_boxed_slice()).as_ptr()
}

fn interface(
    name: &'static CStr,
    version: c_int,
    methods: (*const WlMessage, c_int),
    events: (*const WlMessage, c_int),
) -> *const WlInterface {
    Box::leak(Box::new(WlInterface {
        name: name.as_ptr(),
        version,
        method_count: methods.1,
        methods: methods.0,
        event_count: events.1,
        events: events.0,
    }))
}

fn output() -> *const WlInterface {
    ptr::addr_of!(ffi::wl_output_interface)
}

/// Like [`interface`], for an event table that refers to the interface itself
/// (`zwlr_foreign_toplevel_handle_v1.parent`).
fn interface_with_self(
    name: &'static CStr,
    version: c_int,
    methods: (*const WlMessage, c_int),
    events: impl FnOnce(*const WlInterface) -> (*const WlMessage, c_int),
) -> *const WlInterface {
    let raw = Box::into_raw(Box::new(WlInterface {
        name: name.as_ptr(),
        version,
        method_count: methods.1,
        methods: methods.0,
        event_count: 0,
        events: ptr::null(),
    }));
    let (table, count) = events(raw as *const WlInterface);
    unsafe {
        (*raw).events = table;
        (*raw).event_count = count;
    }
    raw
}

struct Protocols {
    list: *const WlInterface,
    manager: *const WlInterface,
}

impl Protocols {
    fn new() -> Self {
        let list_handle = interface(
            c"ext_foreign_toplevel_handle_v1",
            1,
            table(&[(c"destroy", c"", ptr::null())]),
            table(&[
                (c"closed", c"", ptr::null()),
                (c"done", c"", ptr::null()),
                (c"title", c"s", ptr::null()),
                (c"app_id", c"s", ptr::null()),
                (c"identifier", c"s", ptr::null()),
            ]),
        );
        let list = interface(
            c"ext_foreign_toplevel_list_v1",
            1,
            table(&[(c"stop", c"", ptr::null()), (c"destroy", c"", ptr::null())]),
            table(&[
                (c"toplevel", c"n", types(&[list_handle])),
                (c"finished", c"", ptr::null()),
            ]),
        );
        let manager_handle = interface_with_self(
            c"zwlr_foreign_toplevel_handle_v1",
            3,
            table(&[
                (c"activate", c"o", types(&[output()])),
                (c"close", c"", ptr::null()),
                (c"set_maximized", c"", ptr::null()),
                (c"unset_maximized", c"", ptr::null()),
                (c"set_minimized", c"", ptr::null()),
                (c"unset_minimized", c"", ptr::null()),
                (c"set_fullscreen", c"?o", types(&[output()])),
                (c"unset_fullscreen", c"", ptr::null()),
                (c"destroy", c"", ptr::null()),
            ]),
            |manager_handle| {
                table(&[
                    (c"title", c"s", ptr::null()),
                    (c"app_id", c"s", ptr::null()),
                    (c"output_enter", c"o", types(&[output()])),
                    (c"output_leave", c"o", types(&[output()])),
                    (c"state", c"a", ptr::null()),
                    (c"done", c"", ptr::null()),
                    (c"closed", c"", ptr::null()),
                    (c"parent", c"?o", types(&[manager_handle])),
                ])
            },
        );
        let manager = interface(
            c"zwlr_foreign_toplevel_manager_v1",
            3,
            table(&[(c"stop", c"", ptr::null())]),
            table(&[
                (c"toplevel", c"n", types(&[manager_handle])),
                (c"finished", c"", ptr::null()),
            ]),
        );
        Self { list, manager }
    }
}

#[derive(Default)]
struct View {
    title: String,
    app_id: String,
    activated: bool,
}

#[derive(Default)]
struct Handle {
    title: String,
    app_id: String,
    identifier: Option<String>,
}

struct State {
    protocols: Protocols,
    registry_impl: [*const c_void; 2],
    list_impl: [*const c_void; 2],
    list_handle_impl: [*const c_void; 5],
    manager_impl: [*const c_void; 2],
    manager_handle_impl: [*const c_void; 8],
    list: Option<Proxy>,
    manager: Option<Proxy>,
    views: HashMap<usize, View>,
    handles: HashMap<usize, Handle>,
}

impl State {
    fn new() -> Self {
        Self {
            protocols: Protocols::new(),
            registry_impl: [
                on_global as *const () as *const c_void,
                ignore2 as *const () as *const c_void,
            ],
            list_impl: [
                on_list_toplevel as *const () as *const c_void,
                ignore2 as *const () as *const c_void,
            ],
            list_handle_impl: [
                ignore2 as *const () as *const c_void,
                ignore2 as *const () as *const c_void,
                on_handle_title as *const () as *const c_void,
                on_handle_app_id as *const () as *const c_void,
                on_handle_identifier as *const () as *const c_void,
            ],
            manager_impl: [
                on_manager_toplevel as *const () as *const c_void,
                ignore2 as *const () as *const c_void,
            ],
            manager_handle_impl: [
                on_view_title as *const () as *const c_void,
                on_view_app_id as *const () as *const c_void,
                ignore3 as *const () as *const c_void,
                ignore3 as *const () as *const c_void,
                on_view_state as *const () as *const c_void,
                ignore2 as *const () as *const c_void,
                ignore2 as *const () as *const c_void,
                ignore3 as *const () as *const c_void,
            ],
            list: None,
            manager: None,
            views: HashMap::new(),
            handles: HashMap::new(),
        }
    }
}

fn state<'a>(data: *mut c_void) -> &'a mut State {
    unsafe { &mut *(data as *mut State) }
}

unsafe fn bind(registry: Proxy, name: u32, iface: *const WlInterface, version: u32) -> Proxy {
    unsafe {
        ffi::wl_proxy_marshal_flags(
            registry,
            0, // wl_registry.bind
            iface,
            version,
            0,
            name,
            (*iface).name,
            version,
            ptr::null_mut::<c_void>(),
        )
    }
}

fn string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}

extern "C" fn ignore2(_data: *mut c_void, _proxy: Proxy) {}

// Listener slots for events this tool has no use for; the third argument is a
// pointer of some kind (object, string or array) in every case here.
extern "C" fn ignore3(_data: *mut c_void, _proxy: Proxy, _arg: Proxy) {}

extern "C" fn on_global(
    data: *mut c_void,
    registry: Proxy,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    let interface = string(interface);
    let state = state(data);
    let is_list = interface.as_bytes() == LIST.to_bytes();
    let (iface, requested, impls) = if is_list {
        (state.protocols.list, 1, state.list_impl.as_ptr())
    } else if interface.as_bytes() == MANAGER.to_bytes() {
        (state.protocols.manager, 3, state.manager_impl.as_ptr())
    } else {
        return;
    };
    let version = version.min(requested);
    unsafe {
        let proxy = bind(registry, name, iface, version);
        ffi::wl_proxy_add_listener(proxy, impls, data);
        if is_list {
            state.list = Some(proxy);
        } else {
            state.manager = Some(proxy);
        }
    }
}

extern "C" fn on_list_toplevel(data: *mut c_void, _list: Proxy, handle: Proxy) {
    let state = state(data);
    state.handles.insert(handle as usize, Handle::default());
    let impls = state.list_handle_impl.as_ptr();
    unsafe { ffi::wl_proxy_add_listener(handle, impls, data) };
}

extern "C" fn on_manager_toplevel(data: *mut c_void, _manager: Proxy, handle: Proxy) {
    let state = state(data);
    state.views.insert(handle as usize, View::default());
    let impls = state.manager_handle_impl.as_ptr();
    unsafe { ffi::wl_proxy_add_listener(handle, impls, data) };
}

extern "C" fn on_handle_title(data: *mut c_void, handle: Proxy, title: *const c_char) {
    let title = string(title);
    if let Some(entry) = state(data).handles.get_mut(&(handle as usize)) {
        entry.title = title;
    }
}

extern "C" fn on_handle_app_id(data: *mut c_void, handle: Proxy, app_id: *const c_char) {
    let app_id = string(app_id);
    if let Some(entry) = state(data).handles.get_mut(&(handle as usize)) {
        entry.app_id = app_id;
    }
}

extern "C" fn on_handle_identifier(data: *mut c_void, handle: Proxy, identifier: *const c_char) {
    let identifier = string(identifier);
    if let Some(entry) = state(data).handles.get_mut(&(handle as usize)) {
        entry.identifier = Some(identifier);
    }
}

extern "C" fn on_view_title(data: *mut c_void, handle: Proxy, title: *const c_char) {
    let title = string(title);
    if let Some(entry) = state(data).views.get_mut(&(handle as usize)) {
        entry.title = title;
    }
}

extern "C" fn on_view_app_id(data: *mut c_void, handle: Proxy, app_id: *const c_char) {
    let app_id = string(app_id);
    if let Some(entry) = state(data).views.get_mut(&(handle as usize)) {
        entry.app_id = app_id;
    }
}

extern "C" fn on_view_state(data: *mut c_void, handle: Proxy, array: *const WlArray) {
    let array = unsafe { &*array };
    let words = unsafe { std::slice::from_raw_parts(array.data as *const u32, array.size / 4) };
    let activated = words.contains(&ACTIVATED);
    if let Some(entry) = state(data).views.get_mut(&(handle as usize)) {
        entry.activated = activated;
    }
}

/// Identifier of the activated toplevel, matched against the list handles.
fn resolve(views: &HashMap<usize, View>, handles: &HashMap<usize, Handle>) -> Option<String> {
    let focused = views.values().find(|view| view.activated)?;
    handles
        .values()
        .find(|handle| handle.title == focused.title && handle.app_id == focused.app_id)?
        .identifier
        .clone()
}

pub fn focused_identifier() -> Result<String, String> {
    let missing = || format!("{} unavailable", LIST.to_string_lossy());
    unsafe {
        let display = ffi::wl_display_connect(ptr::null());
        if display.is_null() {
            return Err("cannot connect to $WAYLAND_DISPLAY".into());
        }
        let state: &'static mut State = Box::leak(Box::new(State::new()));
        let data = state as *mut State as *mut c_void;
        let registry = ffi::wl_proxy_marshal_flags(
            display,
            1, // wl_display.get_registry
            ptr::addr_of!(ffi::wl_registry_interface),
            1,
            0,
            ptr::null_mut::<c_void>(),
        );
        if registry.is_null()
            || ffi::wl_proxy_add_listener(registry, state.registry_impl.as_ptr(), data) != 0
        {
            return Err("cannot reach the compositor registry".into());
        }
        // Globals first, then the toplevel handles, then their properties.
        for _ in 0..3 {
            if ffi::wl_display_roundtrip(display) < 0 {
                return Err("lost the Wayland connection".into());
            }
        }
        if state.list.is_none() {
            return Err(missing());
        }
        if state.manager.is_none() {
            return Err(format!("{} unavailable", MANAGER.to_string_lossy()));
        }
        match resolve(&state.views, &state.handles) {
            Some(identifier) => Ok(identifier),
            None if !state.views.values().any(|view| view.activated) => {
                Err("no focused window".into())
            }
            None => Err("no identifier for the focused window".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(title: &str, app_id: &str, activated: bool) -> View {
        View {
            title: title.into(),
            app_id: app_id.into(),
            activated,
        }
    }

    fn handle(title: &str, app_id: &str, identifier: &str) -> Handle {
        Handle {
            title: title.into(),
            app_id: app_id.into(),
            identifier: Some(identifier.into()),
        }
    }

    #[test]
    fn resolves_the_activated_view_to_its_identifier() {
        let views = HashMap::from([(1, view("a", "foot", false)), (2, view("b", "foot", true))]);
        let handles = HashMap::from([
            (10, handle("a", "foot", "one")),
            (11, handle("b", "foot", "two")),
        ]);
        assert_eq!(resolve(&views, &handles).as_deref(), Some("two"));
    }

    #[test]
    fn no_activated_view_resolves_to_nothing() {
        let views = HashMap::from([(1, view("a", "foot", false))]);
        let handles = HashMap::from([(10, handle("a", "foot", "one"))]);
        assert_eq!(resolve(&views, &handles), None);
    }

    #[test]
    fn focused_view_without_a_matching_handle_resolves_to_nothing() {
        let views = HashMap::from([(1, view("a", "foot", true))]);
        let handles = HashMap::from([(10, handle("b", "foot", "one"))]);
        assert_eq!(resolve(&views, &handles), None);
    }
}
