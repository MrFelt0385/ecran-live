// Probe : SLEventPostToPid est-il résolu ?
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
}

type PostToPidFn = unsafe extern "C" fn(i32, *mut c_void);

fn find_sym(name: &[u8]) -> Option<*mut c_void> {
    static LOADED: OnceLock<()> = OnceLock::new();
    LOADED.get_or_init(|| {
        let path = b"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight\0";
        unsafe { libc::dlopen(path.as_ptr() as *const c_char, libc::RTLD_LAZY | libc::RTLD_GLOBAL); }
    });
    let ptr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char) };
    if ptr.is_null() { None } else { Some(ptr) }
}

fn main() {
    let post = find_sym(b"SLEventPostToPid\0");
    println!("SLEventPostToPid: {}", if post.is_some() { "RESOLU" } else { "absent" });
    let auth = find_sym(b"SLEventSetAuthenticationMessage\0");
    println!("SLEventSetAuthenticationMessage: {}", if auth.is_some() { "RESOLU" } else { "absent" });
    unsafe extern "C" { fn objc_getClass(name: *const c_char) -> *mut c_void; }
    let name = b"SLSEventAuthenticationMessage\0";
    let cls = unsafe { objc_getClass(name.as_ptr() as *const c_char) };
    println!("SLSEventAuthenticationMessage class: {}", if cls.is_null() { "absent" } else { "present" });
}
