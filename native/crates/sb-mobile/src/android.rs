//! Android only: hands the library the app's Java side, which it needs for one thing, reading the phone's own
//! DNS servers (`sb_link::endpoint::install_android_context` says why).
//!
//! The rest of this crate is called through UniFFI, which passes values and has no way to pass the Java VM. So
//! this one call is a plain JNI method instead. The Kotlin side is
//! `com.glazkov.switchboard.android.data.NativeContext`:  (leakscan: allow, the app's package)
//!
//! ```kotlin
//! object NativeContext {
//!     init { System.loadLibrary("sb_mobile") }
//!     @JvmStatic external fun install(context: Context): Boolean
//! }
//! ```
//!
//! and the app calls `NativeContext.install(applicationContext)` before it makes its `HomeLink`. An app that
//! never calls it still works: names are then looked up at public resolvers, as before.

use jni_sys::{JNI_OK, JNIEnv, JavaVM, jboolean, jclass, jobject};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set once the context is installed (or being installed): a second install would be refused by the crate that
/// keeps it, with a panic.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// `NativeContext.install(context)`. True when the library has the context, now or from an earlier call.
///
/// # Safety
///
/// Called by the Java VM as a JNI method: `env` is the calling thread's environment and `context` a live
/// reference to an `android.content.Context`.
#[unsafe(no_mangle)]
// leakscan: allow next line (the name the Android app's package gives it)
pub unsafe extern "system" fn Java_com_glazkov_switchboard_android_data_NativeContext_install(
    env: *mut JNIEnv,
    _class: jclass,
    context: jobject,
) -> jboolean {
    if env.is_null() || context.is_null() {
        return false;
    }
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return true;
    }
    unsafe {
        let calls = (**env).v1_1;
        let mut vm: *mut JavaVM = std::ptr::null_mut();
        // A global reference, never released: the application Context lives as long as the process does.
        let kept = if (calls.GetJavaVM)(env, &mut vm) == JNI_OK && !vm.is_null() {
            (calls.NewGlobalRef)(env, context)
        } else {
            std::ptr::null_mut()
        };
        if kept.is_null() {
            INSTALLED.store(false, Ordering::SeqCst);
            return false;
        }
        sb_link::endpoint::install_android_context(vm.cast::<c_void>(), kept.cast::<c_void>());
    }
    true
}
