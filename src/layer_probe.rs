// FaceHAL/src/layer_probe.rs
//
// Standalone probe: can this HAL process put pixels on the screen on its own?
//
// On the current ROM system_server cannot mint a HIDL token
// (android.hidl.token@1.0::ITokenManager is absent), so ISession::enroll sends
// `fd=0 ints=[0]` and surface_bridge has nothing to decode. This probe tests the
// only HAL-side route left: create a SurfaceFlinger layer of our own, then
// dequeue, fill and post a buffer into it.
//
// It is expected to work only because the HAL runs as uid 1000 (system):
// SurfaceFlinger::createLayer keeps a parentless layer in the hierarchy when
// callingThreadHasUnscopedSurfaceFlingerAccess() returns true, and that function
// short-circuits to true for AID_SYSTEM and AID_GRAPHICS.
//
// Nothing here is linked at build time. The vendor binary may not link libgui.so,
// so every symbol is resolved with dlopen/dlsym and each miss is logged by name.
// A ROM whose libgui does not match these signatures therefore reports exactly
// which symbol it lacks instead of dying with an unresolved-symbol abort.
//
// Not shipped: scripts/stage_vendor.sh installs only face-hal-service.

mod logging;

use std::ffi::c_void;
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::time::Duration;

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

const LAYER_NAME: &[u8] = b"FaceHalPreviewProbe\0";
const WIDTH: i32 = 480;
const HEIGHT: i32 = 640;
const PIXEL_FORMAT_RGBA_8888: i32 = 1;
const DISPLAY_SECONDS: u64 = 30;

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;

    fn ANativeWindow_setBuffersGeometry(
        window: *mut c_void,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;
    fn ANativeWindow_lock(
        window: *mut c_void,
        buffer: *mut ANativeWindow_Buffer,
        dirty_bounds: *mut c_void,
    ) -> i32;
    fn ANativeWindow_unlockAndPost(window: *mut c_void) -> i32;
}

#[repr(C)]
struct ANativeWindow_Buffer {
    width: i32,
    height: i32,
    stride: i32,
    format: i32,
    bits: *mut c_void,
    reserved: [u32; 6],
}

/// Over-sized, over-aligned scratch space for a C++ object that is created by its
/// own constructor. The real objects live inside libgui and their sizes are not
/// part of any header this crate can see; a constructor only ever writes within
/// its own sizeof, so a larger buffer is safe.
#[repr(C, align(16))]
struct Blob([u8; 1024]);

// Method table, all member functions so the surface control pointer is `this`.
type FnGetDefault = unsafe extern "C" fn() -> *mut c_void;
type FnCreateSurfaceChecked = unsafe extern "C" fn(
    *mut c_void,       // this (SurfaceComposerClient*)
    *const c_void,     // const String8& name
    u32,               // w
    u32,               // h
    i32,               // PixelFormat
    *mut *mut c_void,  // sp<SurfaceControl>*
    i32,               // flags
    *const c_void,     // const sp<IBinder>& parent
    *mut c_void,       // LayerMetadata (passed indirectly: non-trivial)
    *mut u32,          // outTransformHint
) -> i32;
type FnGetSurface = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type FnCtor0 = unsafe extern "C" fn(*mut c_void);
type FnString8Ctor = unsafe extern "C" fn(*mut c_void, *const c_char);
type FnTxSetLayer = unsafe extern "C" fn(*mut c_void, *const *mut c_void, i32) -> *mut c_void;
type FnTxSetPosition =
    unsafe extern "C" fn(*mut c_void, *const *mut c_void, f32, f32) -> *mut c_void;
type FnTxApply = unsafe extern "C" fn(*mut c_void, bool, bool) -> i32;

fn dlopen_first(names: &[&[u8]]) -> Option<*mut c_void> {
    for name in names {
        let handle = unsafe { dlopen(name.as_ptr() as *const c_char, RTLD_NOW | RTLD_GLOBAL) };
        if !handle.is_null() {
            return Some(handle);
        }
    }
    None
}

fn symbol(handle: *mut c_void, name: &[u8]) -> *mut c_void {
    unsafe { dlsym(handle, name.as_ptr() as *const c_char) }
}

/// Resolve one symbol, logging it by name when the ROM does not export it.
fn resolve(handle: *mut c_void, name: &[u8]) -> Option<*mut c_void> {
    let sym = symbol(handle, name);
    if sym.is_null() {
        logging::error(&format!(
            "layer-probe: missing symbol {}",
            String::from_utf8_lossy(&name[..name.len() - 1])
        ));
        None
    } else {
        logging::info(&format!(
            "layer-probe: resolved {} at {:#x}",
            String::from_utf8_lossy(&name[..name.len() - 1]),
            sym as usize
        ));
        Some(sym)
    }
}

fn main() {
    logging::init();
    logging::info("layer-probe: start");

    let Some(gui) = dlopen_first(&[
        b"libgui.so\0",
        b"/system/lib64/libgui.so\0",
        b"/system/lib64/libgui_aidl.so\0",
    ]) else {
        logging::error("layer-probe: could not dlopen libgui.so");
        return;
    };

    // Android 15 libgui mangled names. createSurfaceChecked's metadata parameter
    // is `gui::LayerMetadata`, a class with a non-trivial copy constructor, so the
    // Itanium ABI passes it by invisible reference and Rust sees a plain pointer.
    const SYM_GET_DEFAULT: &[u8] = b"_ZN7android21SurfaceComposerClient10getDefaultEv\0";
    const SYM_CREATE_CHECKED: &[u8] = b"_ZN7android21SurfaceComposerClient20createSurfaceCheckedERKNS_7String8EjjiPNS_2spINS_14SurfaceControlEEEiRKNS4_INS_7IBinderEEENS_3gui13LayerMetadataEPj\0";
    const SYM_GET_SURFACE: &[u8] = b"_ZN7android14SurfaceControl10getSurfaceEv\0";
    const SYM_STRING8_CTOR: &[u8] = b"_ZN7android7String8C1EPKc\0";
    const SYM_STRING8_CTOR2: &[u8] = b"_ZN7android7String8C2EPKc\0";
    const SYM_METADATA_CTOR: &[u8] = b"_ZN7android3gui13LayerMetadataC1Ev\0";
    const SYM_TX_CTOR: &[u8] = b"_ZN7android21SurfaceComposerClient11TransactionC1Ev\0";
    const SYM_TX_SET_LAYER: &[u8] =
        b"_ZN7android21SurfaceComposerClient11Transaction8setLayerERKNS_2spINS_14SurfaceControlEEEi\0";
    const SYM_TX_SET_POSITION: &[u8] =
        b"_ZN7android21SurfaceComposerClient11Transaction11setPositionERKNS_2spINS_14SurfaceControlEEEff\0";
    const SYM_TX_APPLY: &[u8] = b"_ZN7android21SurfaceComposerClient11Transaction5applyEbb\0";

    let get_default = resolve(gui, SYM_GET_DEFAULT).map(|s| unsafe {
        std::mem::transmute::<*mut c_void, FnGetDefault>(s)
    });
    let create_checked = resolve(gui, SYM_CREATE_CHECKED).map(|s| unsafe {
        std::mem::transmute::<*mut c_void, FnCreateSurfaceChecked>(s)
    });
    let get_surface = resolve(gui, SYM_GET_SURFACE).map(|s| unsafe {
        std::mem::transmute::<*mut c_void, FnGetSurface>(s)
    });
    let string8_ctor = resolve(gui, SYM_STRING8_CTOR)
        .or_else(|| resolve(gui, SYM_STRING8_CTOR2))
        .map(|s| unsafe { std::mem::transmute::<*mut c_void, FnString8Ctor>(s) });
    let metadata_ctor = resolve(gui, SYM_METADATA_CTOR).map(|s| unsafe {
        std::mem::transmute::<*mut c_void, FnCtor0>(s)
    });

    let (Some(get_default), Some(create_checked), Some(get_surface)) =
        (get_default, create_checked, get_surface)
    else {
        logging::error("layer-probe: libgui lacks the surface creation entry points, stopping");
        return;
    };
    let (Some(string8_ctor), Some(metadata_ctor)) = (string8_ctor, metadata_ctor) else {
        logging::error("layer-probe: libgui lacks String8/LayerMetadata constructors, stopping");
        return;
    };

    let client = unsafe { get_default() };
    if client.is_null() {
        logging::error("layer-probe: SurfaceComposerClient::getDefault() returned null");
        return;
    }
    logging::info(&format!("layer-probe: client at {:#x}", client as usize));

    let mut name = Blob([0u8; 1024]);
    unsafe {
        string8_ctor(
            name.0.as_mut_ptr() as *mut c_void,
            LAYER_NAME.as_ptr() as *const c_char,
        )
    };

    let mut metadata = Blob([0u8; 1024]);
    unsafe { metadata_ctor(metadata.0.as_mut_ptr() as *mut c_void) };

    // `nullptr` cannot be used for the parent handle: the parameter is a
    // `const sp<IBinder>&` and SF copy-assigns from it, so it has to point at a
    // real, empty sp. A zeroed sp is an empty sp (its only member is the pointer).
    let no_parent = Blob([0u8; 1024]);

    let mut control: *mut c_void = ptr::null_mut();
    let status = unsafe {
        create_checked(
            client,
            name.0.as_ptr() as *const c_void,
            WIDTH as u32,
            HEIGHT as u32,
            PIXEL_FORMAT_RGBA_8888,
            &mut control as *mut *mut c_void,
            0,
            no_parent.0.as_ptr() as *const c_void,
            metadata.0.as_mut_ptr() as *mut c_void,
            ptr::null_mut(),
        )
    };
    logging::info(&format!(
        "layer-probe: createSurfaceChecked ret={} control={:#x}",
        status, control as usize
    ));
    if status != 0 || control.is_null() {
        logging::error("layer-probe: SurfaceFlinger refused to create the layer");
        return;
    }

    // Put it in front of the app windows and roughly where an enrollment preview
    // sits on a portrait phone. WindowManager hands app windows z values in the
    // tens of thousands, so one million is comfortably above them.
    let transaction = resolve(gui, SYM_TX_CTOR);
    let set_layer = resolve(gui, SYM_TX_SET_LAYER);
    let set_position = resolve(gui, SYM_TX_SET_POSITION);
    let apply = resolve(gui, SYM_TX_APPLY);
    if let (Some(tx_ctor), Some(set_layer), Some(set_position), Some(apply)) =
        (transaction, set_layer, set_position, apply)
    {
        let tx_ctor: FnCtor0 = unsafe { std::mem::transmute(tx_ctor) };
        let set_layer: FnTxSetLayer = unsafe { std::mem::transmute(set_layer) };
        let set_position: FnTxSetPosition = unsafe { std::mem::transmute(set_position) };
        let apply: FnTxApply = unsafe { std::mem::transmute(apply) };

        let mut tx = Blob([0u8; 1024]);
        unsafe {
            tx_ctor(tx.0.as_mut_ptr() as *mut c_void);
            set_layer(
                tx.0.as_mut_ptr() as *mut c_void,
                &control as *const *mut c_void,
                1_000_000,
            );
            set_position(
                tx.0.as_mut_ptr() as *mut c_void,
                &control as *const *mut c_void,
                120.0,
                360.0,
            );
        }
        let ret = unsafe { apply(tx.0.as_mut_ptr() as *mut c_void, true, false) };
        logging::info(&format!("layer-probe: transaction apply ret={}", ret));
    } else {
        logging::error("layer-probe: no Transaction entry points, layer stays at z=0");
    }

    let surface = unsafe { get_surface(control) };
    if surface.is_null() {
        logging::error("layer-probe: SurfaceControl::getSurface() returned null");
        return;
    }
    logging::info(&format!(
        "layer-probe: Surface at {:#x}, drawing {}x{}",
        surface as usize, WIDTH, HEIGHT
    ));

    // Surface derives from ANativeWindow, so its pointer is the producer window.
    let geom = unsafe { ANativeWindow_setBuffersGeometry(surface, WIDTH, HEIGHT, PIXEL_FORMAT_RGBA_8888) };
    logging::info(&format!("layer-probe: setBuffersGeometry ret={}", geom));

    let mut buffer = ANativeWindow_Buffer {
        width: 0,
        height: 0,
        stride: 0,
        format: 0,
        bits: ptr::null_mut(),
        reserved: [0; 6],
    };
    let locked = unsafe { ANativeWindow_lock(surface, &mut buffer, ptr::null_mut()) };
    logging::info(&format!(
        "layer-probe: lock ret={} {}x{} stride={} bits={:#x}",
        locked,
        buffer.width,
        buffer.height,
        buffer.stride,
        buffer.bits as usize
    ));
    if locked == 0 && !buffer.bits.is_null() {
        // Opaque magenta: impossible to mistake for a black preview.
        let stride = buffer.stride.max(buffer.width) as usize;
        unsafe {
            let base = buffer.bits as *mut u8;
            for y in 0..buffer.height as usize {
                let row = base.add(y * stride * 4);
                for x in 0..buffer.width as usize {
                    let px = row.add(x * 4);
                    *px = 0xff;
                    *px.add(1) = 0x00;
                    *px.add(2) = 0xff;
                    *px.add(3) = 0xff;
                }
            }
        }
        let posted = unsafe { ANativeWindow_unlockAndPost(surface) };
        logging::info(&format!("layer-probe: unlockAndPost ret={}", posted));
    }

    logging::info(&format!(
        "layer-probe: layer alive for {}s, watch the screen now",
        DISPLAY_SECONDS
    ));
    std::thread::sleep(Duration::from_secs(DISPLAY_SECONDS));
    logging::info("layer-probe: done");
}
