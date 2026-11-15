// FaceHAL/src/preview_layer.rs
//
// Enrollment preview drawn by the HAL itself.
//
// The framework is supposed to hand the HAL a surface to draw into, but on this
// ROM system_server has no android.hidl.token@1.0::ITokenManager, so `enroll`
// arrives with `fd=0 ints=[0]` and there is nothing to decode. What is left is a
// SurfaceFlinger layer of our own: `SurfaceComposerClient::createSurfaceChecked`
// keeps a parentless layer in the hierarchy when the caller has unscoped
// SurfaceFlinger access, which is true for AID_SYSTEM (uid 1000, what this HAL
// runs as).
//
// Two things the prototype probe had to learn the hard way are baked in here:
//
// * `sp<T>` is returned through the hidden result pointer in x8, so those calls
//   are declared as returning a struct larger than two registers.
// * The `ANativeWindow_*` C helpers are unusable on this ROM. Their dispatch
//   lands in libgui's deprecated queueBuffer hook with a status_t where a buffer
//   pointer belongs, so the producer API (Surface::dequeueBuffer, GraphicBuffer
//   ::lock, Surface::queueBuffer) is called through dlsym instead.
//
// Nothing here is linked at build time: the HAL does not link libgui, and a ROM
// whose libgui disagrees with these signatures reports the missing name instead
// of failing to start.

use std::ffi::c_void;
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::logging;

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

const PIXEL_FORMAT_RGBA_8888: i32 = 1;
const GRALLOC_USAGE_SW_RW_OFTEN: u32 = 0x3;
const NATIVE_WINDOW_API_CPU: i32 = 2;

/// A 60 Hz camera would otherwise spend every frame on a copy that SurfaceFlinger
/// throws away. 30 fps is what the preview needs and what the CPU can afford.
const FRAME_INTERVAL: Duration = Duration::from_millis(33);

extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
}

/// Result slot for a function returning `sp<T>` by value: the callee writes the
/// reference-counted pointer through the hidden result pointer in x8, and a Rust
/// return type larger than two registers makes the compiler pass exactly that.
#[repr(C)]
struct SpOut {
    slot: [u8; 32],
}

impl SpOut {
    fn ptr(&self) -> *mut c_void {
        let mut p: *mut c_void = ptr::null_mut();
        unsafe {
            ptr::copy_nonoverlapping(
                self.slot.as_ptr(),
                &mut p as *mut *mut c_void as *mut u8,
                std::mem::size_of::<*mut c_void>(),
            );
        }
        p
    }
}

/// Scratch space for a C++ object built by its own constructor. The real sizes
/// live inside libgui; a constructor only writes within its own sizeof.
#[repr(C, align(16))]
struct Blob([u8; 4096]);

impl Blob {
    fn new() -> Self {
        Blob([0u8; 4096])
    }

    fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr() as *mut c_void
    }
}

type FnGetDefault = unsafe extern "C" fn() -> SpOut;
type FnCreateSurfaceChecked = unsafe extern "C" fn(
    *mut c_void,       // this (SurfaceComposerClient*)
    *const c_void,     // const String8& name
    u32,               // w
    u32,               // h
    i32,               // PixelFormat
    *mut *mut c_void,  // sp<SurfaceControl>*
    i32,               // flags
    *const c_void,     // const sp<IBinder>& parent
    *const c_void,     // const gui::LayerMetadata&
    *mut u32,          // outTransformHint
) -> i32;
type FnGetSurface = unsafe extern "C" fn(*mut c_void) -> SpOut;
type FnCtor0 = unsafe extern "C" fn(*mut c_void);
type FnString8Ctor = unsafe extern "C" fn(*mut c_void, *const c_char);
type FnSetLayer = unsafe extern "C" fn(*mut c_void, *const *mut c_void, i32) -> *mut c_void;
type FnSetPosition =
    unsafe extern "C" fn(*mut c_void, *const *mut c_void, f32, f32) -> *mut c_void;
type FnReparent = unsafe extern "C" fn(
    *mut c_void,
    *const *mut c_void,
    *const *mut c_void,
) -> *mut c_void;
type FnApply = unsafe extern "C" fn(*mut c_void, bool, bool) -> i32;
type FnSetFlags = unsafe extern "C" fn(*mut c_void, *const *mut c_void, u32, u32) -> *mut c_void;
type FnSetDims = unsafe extern "C" fn(*mut c_void, u32, u32) -> i32;
type FnSetFormat = unsafe extern "C" fn(*mut c_void, i32) -> i32;
type FnConnect = unsafe extern "C" fn(*mut c_void, i32) -> i32;
type FnDequeue = unsafe extern "C" fn(*mut c_void, *mut *mut c_void, *mut *mut c_void) -> i32;
type FnGraphicBufferLock =
    unsafe extern "C" fn(*mut c_void, u32, *mut *mut c_void, *mut i32, *mut i32) -> i32;
type FnGraphicBufferUnlock = unsafe extern "C" fn(*mut c_void) -> i32;
type FnQueue =
    unsafe extern "C" fn(*mut c_void, *const *mut c_void, *const *mut c_void, *mut c_void) -> i32;

const SYM_GET_DEFAULT: &[u8] = b"_ZN7android21SurfaceComposerClient10getDefaultEv\0";
const SYM_CREATE_CHECKED: &[u8] = b"_ZN7android21SurfaceComposerClient20createSurfaceCheckedERKNS_7String8EjjiPNS_2spINS_14SurfaceControlEEEiRKNS4_INS_7IBinderEEERKNS_3gui13LayerMetadataEPj\0";
const SYM_GET_SURFACE: &[u8] = b"_ZN7android14SurfaceControl10getSurfaceEv\0";
const SYM_STRING8_CTOR: &[u8] = b"_ZN7android7String8C1EPKc\0";
const SYM_STRING8_CTOR2: &[u8] = b"_ZN7android7String8C2EPKc\0";
const SYM_METADATA_CTOR: &[u8] = b"_ZN7android3gui13LayerMetadataC1Ev\0";
const SYM_TX_CTOR: &[u8] = b"_ZN7android21SurfaceComposerClient11TransactionC1Ev\0";
const SYM_TX_SET_LAYER: &[u8] =
    b"_ZN7android21SurfaceComposerClient11Transaction8setLayerERKNS_2spINS_14SurfaceControlEEEi\0";
const SYM_TX_SET_POSITION: &[u8] =
    b"_ZN7android21SurfaceComposerClient11Transaction11setPositionERKNS_2spINS_14SurfaceControlEEEff\0";
const SYM_TX_REPARENT: &[u8] = b"_ZN7android21SurfaceComposerClient11Transaction8reparentERKNS_2spINS_14SurfaceControlEEES6_\0";
const SYM_TX_SET_FLAGS: &[u8] =
    b"_ZN7android21SurfaceComposerClient11Transaction8setFlagsERKNS_2spINS_14SurfaceControlEEEjj\0";
const SYM_TX_APPLY: &[u8] = b"_ZN7android21SurfaceComposerClient11Transaction5applyEbb\0";
const SYM_SET_DIMS: &[u8] = b"_ZN7android7Surface20setBuffersDimensionsEjj\0";
const SYM_SET_FORMAT: &[u8] = b"_ZN7android7Surface16setBuffersFormatEi\0";
const SYM_CONNECT: &[u8] = b"_ZN7android7Surface7connectEi\0";
const SYM_DEQUEUE: &[u8] =
    b"_ZN7android7Surface13dequeueBufferEPNS_2spINS_13GraphicBufferEEEPNS1_INS_5FenceEEE\0";
const SYM_GB_LOCK: &[u8] = b"_ZN7android13GraphicBuffer4lockEjPPvPiS3_\0";
const SYM_GB_UNLOCK: &[u8] = b"_ZN7android13GraphicBuffer6unlockEv\0";
const SYM_QUEUE: &[u8] = b"_ZN7android7Surface11queueBufferERKNS_2spINS_13GraphicBufferEEERKNS1_INS_5FenceEEEPNS_24SurfaceQueueBufferOutputE\0";

/// Everything resolved out of libgui once, at the first enroll that needs it.
struct Api {
    get_default: FnGetDefault,
    create_checked: FnCreateSurfaceChecked,
    get_surface: FnGetSurface,
    string8_ctor: FnString8Ctor,
    metadata_ctor: FnCtor0,
    tx_ctor: FnCtor0,
    tx_set_layer: FnSetLayer,
    tx_set_position: FnSetPosition,
    tx_reparent: Option<FnReparent>,
    tx_set_flags: Option<FnSetFlags>,
    tx_apply: FnApply,
    set_dims: FnSetDims,
    set_format: FnSetFormat,
    connect: FnConnect,
    dequeue: FnDequeue,
    gb_lock: FnGraphicBufferLock,
    gb_unlock: FnGraphicBufferUnlock,
    queue: FnQueue,
}

fn symbol(handle: *mut c_void, name: &[u8]) -> *mut c_void {
    unsafe { dlsym(handle, name.as_ptr() as *const c_char) }
}

fn resolve(handle: *mut c_void, name: &[u8]) -> Option<*mut c_void> {
    let sym = symbol(handle, name);
    if sym.is_null() {
        logging::error(&format!(
            "preview: libgui is missing {}",
            String::from_utf8_lossy(&name[..name.len() - 1])
        ));
        return None;
    }
    Some(sym)
}

fn load_api() -> Option<Api> {
    // A vendor process (this HAL) runs in an isolated linker namespace: the
    // system-only libgui may simply not be reachable. dlopen then fails without
    // printing anything of ours, which looks exactly like "nothing happened", so
    // the reason is logged here instead of being swallowed.
    let candidates: [&[u8]; 4] = [
        b"libgui.so\0",
        b"/system/lib64/libgui.so\0",
        b"/vendor/lib64/libgui.so\0",
        b"/vendor/lib64/hw/libgui.so\0",
    ];
    let mut gui: *mut c_void = ptr::null_mut();
    for name in candidates {
        let handle = unsafe { dlopen(name.as_ptr() as *const c_char, RTLD_NOW | RTLD_GLOBAL) };
        if !handle.is_null() {
            logging::info(&format!(
                "preview: loaded {}",
                String::from_utf8_lossy(&name[..name.len() - 1])
            ));
            gui = handle;
            break;
        }
        let err = unsafe { dlerror() };
        let detail = if err.is_null() {
            "no dlerror detail".to_string()
        } else {
            unsafe { std::ffi::CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned()
        };
        logging::error(&format!(
            "preview: dlopen({}) failed: {}",
            String::from_utf8_lossy(&name[..name.len() - 1]),
            detail
        ));
    }
    if gui.is_null() {
        logging::error("preview: libgui is unreachable from this process, no layer");
        return None;
    }

    unsafe {
        Some(Api {
            get_default: std::mem::transmute(resolve(gui, SYM_GET_DEFAULT)?),
            create_checked: std::mem::transmute(resolve(gui, SYM_CREATE_CHECKED)?),
            get_surface: std::mem::transmute(resolve(gui, SYM_GET_SURFACE)?),
            string8_ctor: std::mem::transmute(
                resolve(gui, SYM_STRING8_CTOR).or_else(|| resolve(gui, SYM_STRING8_CTOR2))?,
            ),
            metadata_ctor: std::mem::transmute(resolve(gui, SYM_METADATA_CTOR)?),
            tx_ctor: std::mem::transmute(resolve(gui, SYM_TX_CTOR)?),
            tx_set_layer: std::mem::transmute(resolve(gui, SYM_TX_SET_LAYER)?),
            tx_set_position: std::mem::transmute(resolve(gui, SYM_TX_SET_POSITION)?),
            tx_reparent: resolve(gui, SYM_TX_REPARENT).map(|s| std::mem::transmute(s)),
            tx_set_flags: resolve(gui, SYM_TX_SET_FLAGS).map(|s| std::mem::transmute(s)),
            tx_apply: std::mem::transmute(resolve(gui, SYM_TX_APPLY)?),
            set_dims: std::mem::transmute(resolve(gui, SYM_SET_DIMS)?),
            set_format: std::mem::transmute(resolve(gui, SYM_SET_FORMAT)?),
            connect: std::mem::transmute(resolve(gui, SYM_CONNECT)?),
            dequeue: std::mem::transmute(resolve(gui, SYM_DEQUEUE)?),
            gb_lock: std::mem::transmute(resolve(gui, SYM_GB_LOCK)?),
            gb_unlock: std::mem::transmute(resolve(gui, SYM_GB_UNLOCK)?),
            queue: std::mem::transmute(resolve(gui, SYM_QUEUE)?),
        })
    }
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(load_api).as_ref()
}

/// One layer, one connected CPU producer, one frame at a time.
pub struct PreviewLayer {
    api: &'static Api,
    control: *mut c_void,
    surface: *mut c_void,
    width: i32,
    height: i32,
    frames: u32,
    last_post: Option<Instant>,
}

// Raw pointers into libgui's heap. The single owner is the mutex below, and the
// HAL only ever renders from the camera callback thread.
unsafe impl Send for PreviewLayer {}

impl PreviewLayer {
    fn create(width: i32, height: i32, x: f32, y: f32, z: i32) -> Option<PreviewLayer> {
        let api = api()?;
        let client = unsafe { (api.get_default)() }.ptr();
        if client.is_null() {
            logging::error("preview: SurfaceComposerClient::getDefault() returned null");
            return None;
        }

        let name = Blob::new();
        unsafe {
            (api.string8_ctor)(
                name.as_ptr(),
                b"FaceHalEnrollPreview\0".as_ptr() as *const c_char,
            )
        };
        let metadata = Blob::new();
        unsafe { (api.metadata_ctor)(metadata.as_ptr()) };

        // The parent parameter is a `const sp<IBinder>&`, so it has to point at a
        // real empty sp rather than be null.
        let no_parent = Blob::new();
        let mut control: *mut c_void = ptr::null_mut();
        let status = unsafe {
            (api.create_checked)(
                client,
                name.as_ptr(),
                width as u32,
                height as u32,
                PIXEL_FORMAT_RGBA_8888,
                &mut control,
                0,
                no_parent.as_ptr(),
                metadata.as_ptr(),
                ptr::null_mut(),
            )
        };
        if status != 0 || control.is_null() {
            logging::error(&format!(
                "preview: SurfaceFlinger refused the layer, ret={}",
                status
            ));
            return None;
        }

        let surface = unsafe { (api.get_surface)(control) }.ptr();
        if surface.is_null() {
            logging::error("preview: SurfaceControl::getSurface() returned null");
            return None;
        }

        let tx = Blob::new();
        unsafe {
            (api.tx_ctor)(tx.as_ptr());
            (api.tx_set_layer)(tx.as_ptr(), &control as *const *mut c_void, z);
            (api.tx_set_position)(tx.as_ptr(), &control as *const *mut c_void, x, y);
            (api.tx_apply)(tx.as_ptr(), true, false);
        }

        // The Surface has a producer but is not connected to its BufferQueue yet;
        // dequeueBuffer answers NO_INIT until it is.
        unsafe {
            let dims = (api.set_dims)(surface, width as u32, height as u32);
            let format = (api.set_format)(surface, PIXEL_FORMAT_RGBA_8888);
            let connected = (api.connect)(surface, NATIVE_WINDOW_API_CPU);
            logging::info(&format!(
                "preview: layer ready {}x{} at ({}, {}) z={} dims={} format={} connect={}",
                width, height, x, y, z, dims, format, connected
            ));
            if connected != 0 {
                logging::error("preview: layer producer refused the CPU api");
                return None;
            }
        }

        Some(PreviewLayer {
            api,
            control,
            surface,
            width,
            height,
            frames: 0,
            last_post: None,
        })
    }

    fn render_nv21(&mut self, nv21: &[u8], width: i32, height: i32) {
        if width <= 0 || height <= 0 || nv21.len() < (width * height * 3 / 2) as usize {
            return;
        }
        if let Some(last) = self.last_post {
            if last.elapsed() < FRAME_INTERVAL {
                return;
            }
        }

        let mut buffer: *mut c_void = ptr::null_mut();
        let mut fence: *mut c_void = ptr::null_mut();
        let dequeued = unsafe { (self.api.dequeue)(self.surface, &mut buffer, &mut fence) };
        if dequeued != 0 || buffer.is_null() {
            if self.frames == 0 {
                logging::error(&format!("preview: dequeueBuffer ret={}", dequeued));
            }
            return;
        }

        let mut addr: *mut c_void = ptr::null_mut();
        let mut bytes_per_pixel: i32 = 0;
        let mut bytes_per_stride: i32 = 0;
        let locked = unsafe {
            (self.api.gb_lock)(
                buffer,
                GRALLOC_USAGE_SW_RW_OFTEN,
                &mut addr,
                &mut bytes_per_pixel,
                &mut bytes_per_stride,
            )
        };
        if locked != 0 || addr.is_null() {
            logging::error(&format!("preview: GraphicBuffer::lock ret={}", locked));
            return;
        }

        let stride = if bytes_per_stride > 0 {
            bytes_per_stride as usize
        } else {
            (self.width as usize) * 4
        };
        blit_nv21(
            addr as *mut u8,
            self.width,
            self.height,
            stride,
            nv21,
            width,
            height,
        );

        unsafe {
            (self.api.gb_unlock)(buffer);
            let output = Blob::new();
            let queued = (self.api.queue)(
                self.surface,
                &buffer as *const *mut c_void,
                &fence as *const *mut c_void,
                output.as_ptr(),
            );
            if queued != 0 && self.frames == 0 {
                logging::error(&format!("preview: queueBuffer ret={}", queued));
            }
        }

        self.frames += 1;
        self.last_post = Some(Instant::now());
        if self.frames == 1 || self.frames % 30 == 0 {
            logging::info(&format!(
                "preview: rendered frame {} into the HAL layer ({}x{}, stride {})",
                self.frames, self.width, self.height, stride
            ));
        }
    }
}

impl Drop for PreviewLayer {
    fn drop(&mut self) {
        // Reparenting to null detaches a child layer, but this layer was created
        // parentless and sits at the root, where it is a no-op. The hidden flag is
        // what actually takes it off the screen, so set both rather than trust one.
        let tx = Blob::new();
        let nothing: *mut c_void = ptr::null_mut();
        const LAYER_HIDDEN: u32 = 0x1;
        unsafe {
            (self.api.tx_ctor)(tx.as_ptr());
            if let Some(reparent) = self.api.tx_reparent {
                reparent(tx.as_ptr(), &self.control as *const *mut c_void, &nothing);
            }
            if let Some(set_flags) = self.api.tx_set_flags {
                set_flags(
                    tx.as_ptr(),
                    &self.control as *const *mut c_void,
                    LAYER_HIDDEN,
                    LAYER_HIDDEN,
                );
            }
            (self.api.tx_apply)(tx.as_ptr(), true, false);
        }
        logging::info("preview: layer detached and hidden");
    }
}

/// NV21 (the camera's format) into the RGBA buffer behind the layer.
///
/// The sensor is mounted rotated: a 640x480 frame is a portrait scene, so source
/// columns map to destination rows. The image is mirrored, as a selfie preview
/// should be; the algorithm keeps the unmirrored frame.
fn blit_nv21(
    dst: *mut u8,
    dst_w: i32,
    dst_h: i32,
    stride_bytes: usize,
    nv21: &[u8],
    width: i32,
    height: i32,
) {
    let y_plane = nv21;
    let vu_plane = &nv21[(width * height) as usize..];

    let src_pw = height;
    let src_ph = width;

    let (crop_w, crop_h, crop_x0, crop_y0) =
        if dst_w * src_ph > dst_h * src_pw {
            (src_pw, (src_pw * dst_h) / dst_w, 0, (src_ph - (src_pw * dst_h) / dst_w) / 2)
        } else {
            let crop_w = (src_ph * dst_w) / dst_h;
            (crop_w, src_ph, (src_pw - crop_w) / 2, 0)
        };

    let denom_w = (dst_w - 1).max(1);
    let denom_h = (dst_h - 1).max(1);

    let dst_w_usize = (dst_w as usize).min(1080);
    let mut row_y_offs = [0usize; 1080];
    let mut row_uv_offs = [0usize; 1080];
    for dx in 0..dst_w_usize {
        let px = crop_x0 + (dx as i32 * (crop_w - 1)) / denom_w;
        let s_row = px.clamp(0, height - 1) as usize;
        row_y_offs[dx] = s_row * width as usize;
        row_uv_offs[dx] = (s_row >> 1) * width as usize;
    }

    for dy in 0..dst_h {
        let py = crop_y0 + (dy * (crop_h - 1)) / denom_h;
        let sensor_col = ((width - 1) - py).clamp(0, width - 1) as usize;
        let col_uv_base = (sensor_col >> 1) << 1;
        let dst_row = unsafe { dst.add(dy as usize * stride_bytes) as *mut u32 };
        for dx in 0..dst_w_usize {
            let y_val = y_plane[sensor_col + row_y_offs[dx]];
            let uv_idx = row_uv_offs[dx] + col_uv_base;
            let v_val = vu_plane[uv_idx];
            let u_val = vu_plane[uv_idx + 1];
            let (r, g, b) = yuv_to_rgb(y_val, u_val, v_val);
            unsafe {
                *dst_row.add(dst_w_usize - 1 - dx) =
                    0xFF00_0000 | ((b as u32) << 16) | ((g as u32) << 8) | r as u32;
            }
        }
    }
}

#[inline(always)]
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> (i32, i32, i32) {
    let c = (y as i32 - 16).max(0);
    let d = u as i32 - 128;
    let e = v as i32 - 128;
    let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255);
    let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255);
    let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255);
    (r, g, b)
}

static CURRENT: Mutex<Option<PreviewLayer>> = Mutex::new(None);

fn prop(name: &[u8]) -> Option<String> {
    logging::get_prop(name)
}

fn prop_i32(name: &[u8], default: i32) -> i32 {
    prop(name)
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(default)
}

fn prop_f32(name: &[u8], default: f32) -> f32 {
    prop(name)
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(default)
}

/// Geometry is settable so the layer can be lined up with the enrollment UI from
/// the shell, without rebuilding:
///
///   setprop persist.sys.facehal.preview.w 480
///   setprop persist.sys.facehal.preview.h 640
///   setprop persist.sys.facehal.preview.x 120
///   setprop persist.sys.facehal.preview.y 360
///   setprop persist.sys.facehal.preview.z 1000000
pub fn open_current() -> bool {
    let mut slot = match CURRENT.lock() {
        Ok(slot) => slot,
        Err(_) => return false,
    };
    if slot.is_some() {
        return true;
    }

    let width = prop_i32(b"persist.sys.facehal.preview.w\0", 480).clamp(64, 1440);
    let height = prop_i32(b"persist.sys.facehal.preview.h\0", 640).clamp(64, 2560);
    let x = prop_f32(b"persist.sys.facehal.preview.x\0", 120.0);
    let y = prop_f32(b"persist.sys.facehal.preview.y\0", 360.0);
    let z = prop_i32(b"persist.sys.facehal.preview.z\0", 1_000_000);

    match PreviewLayer::create(width, height, x, y, z) {
        Some(layer) => {
            *slot = Some(layer);
            true
        }
        None => false,
    }
}

pub fn render_current(nv21: &[u8], width: i32, height: i32) {
    let mut slot = match CURRENT.lock() {
        Ok(slot) => slot,
        Err(_) => return,
    };
    if let Some(layer) = slot.as_mut() {
        layer.render_nv21(nv21, width, height);
    }
}

pub fn close_current() {
    if let Ok(mut slot) = CURRENT.lock() {
        if slot.is_some() {
            *slot = None;
        }
    }
}
