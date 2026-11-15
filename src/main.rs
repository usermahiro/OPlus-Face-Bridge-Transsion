// FaceHAL/src/main.rs
//
// Entry point for the AIDL face HAL. Registers
// android.hardware.biometrics.face.IFace/default on the vndbinder, which is
// what the OOS (Android 15) framework's FaceService binds to.
//
// The stock X6739 stack only ever exposed the HIDL
// android.hardware.biometrics.face@1.0::IBiometricsFace/default (a stub named
// "...-service.example" plus face.default.so); this binary is the AIDL
// replacement that closes that gap.
mod bindings;
mod camera;
mod face;
mod logging;
mod preview_layer;
mod session;
mod stface;
mod surface_bridge;

use rsbinder::{hub, ProcessState};

use bindings::android::hardware::biometrics::face::IFace::BnFace;
use face::FaceHal;

const SERVICE_NAME: &str = "android.hardware.biometrics.face.IFace/default";

/// init starts this binary from /vendor/bin/hw, which puts the process in the
/// "vendor" linker class: it searches /odm/lib64 and /vendor/lib64, plus a fixed
/// LLNDK allowlist borrowed from the "system" namespace. libgui.so lives only in
/// /system/lib64 and is on neither list, so the enrollment preview layer cannot
/// be created from the vendor class at all -- dlopen is refused before the file
/// is opened.
///
/// The same inode, executed from under /data/local/tmp, lands in the
/// "unrestricted" class, which can load system libraries; that is why the layer
/// probe works when run from a shell. script.sh already keeps a copy of this
/// binary in the tmpfs at /data/local/tmp/facemnt for the bind mount, so the
/// service re-executes that copy and lets it do the work. The vendor-class
/// parent then waits, which keeps init's supervision and the service's process
/// group intact.
///
/// If the exec is refused (SELinux), the HAL keeps running from the vendor path:
/// enrollment, authentication and the algorithm are unaffected, only the preview
/// layer is unavailable.
fn reexec_from_unrestricted() {
    let Ok(exe) = std::fs::read_link("/proc/self/exe") else {
        return;
    };
    let exe = exe.to_string_lossy().into_owned();
    if exe.starts_with("/data/local/tmp/") {
        // Already the unrestricted copy; do not loop.
        return;
    }
    let Some(name) = std::path::Path::new(&exe)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return;
    };
    let candidate = format!("/data/local/tmp/facemnt/{name}");
    // Two names are tried: script.sh leaves the plain copy for the bind mount
    // (labeled hal_face_default_exec, which init needs as an entrypoint) and a
    // second copy with a `.unrestr` suffix labeled vendor_file in case this
    // domain is not allowed to execute its own entrypoint type.
    let candidates = [candidate.clone(), format!("{candidate}.unrestr")];
    let Some(target) = candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .cloned()
    else {
        logging::info(&format!(
            "reexec: {candidate} tidak ada, tetap di linker class vendor"
        ));
        return;
    };

    logging::info(&format!(
        "reexec: menjalankan {target} supaya libgui bisa dimuat"
    ));
    match std::process::Command::new(&target)
        .args(std::env::args_os().skip(1))
        .status()
    {
        Ok(status) => {
            logging::info(&format!("reexec: proses kerja keluar dengan {status}"));
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(e) => logging::error(&format!(
            "reexec: {target} tidak bisa dieksekusi ({e}), lanjut di linker class vendor"
        )),
    }
}

fn main() {
    logging::init();
    logging::init_rsbinder_logger();
    reexec_from_unrestricted();
    logging::info("Face HAL (AIDL, rsbinder) starting");

    // Default driver + a thread pool sized for the camera callback path.
    ProcessState::init_default();
    ProcessState::start_thread_pool();
    surface_bridge::init_hidl_thread_pool();

    let service = FaceHal::new();
    let binder = BnFace::new_binder(service);

    match hub::add_service(SERVICE_NAME, binder.as_binder()) {
        Ok(()) => logging::info(&format!("registered {}", SERVICE_NAME)),
        Err(e) => {
            logging::error(&format!("failed to register {}: {:?}", SERVICE_NAME, e));
            // Nothing useful can happen without the service, so exit and let
            // init's restart policy retry rather than idling silently.
            std::process::exit(1);
        }
    }

    let _ = ProcessState::join_thread_pool();
}
