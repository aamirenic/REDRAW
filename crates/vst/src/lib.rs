//! REDRAW VST hosting: clean-room VST2 (Windows) loading plus VST3 discovery.
//!
//! The ABI structs and opcodes below are declared from the public VST 2.4 ABI
//! description; no SDK or decompiled code is copied. All FFI lives in this
//! crate: callers only see safe Rust. `process` performs no allocation, locks
//! or file I/O after construction. Hosted instances are single-threaded; the
//! engine may move one between control and audio threads only while audio is
//! stopped (`unsafe impl Send` documents that contract).
//!
//! Scope note (D-017): VST2 effect hosting is implemented and verified against a
//! generated mock plug-in. VST3 modules are discovered and signature-probed,
//! but their COM runtime is not hosted yet; out-of-process sandboxing is future
//! work and plugins run in-process. No third-party plug-in ships with REDRAW.
#![allow(unsafe_code)] // FFI boundary; every unsafe block below is documented.
#![deny(unsafe_op_in_unsafe_fn)]

pub use redraw_shared as shared;

use redraw_shared::{Effect, ParamSpec};
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const CRATE_PHASE: u8 = 8;

// ----------------------------------------------------------------------- ABI

/// `AEffect` magic ('VstP').
pub const VST_MAGIC: i32 = 0x5673_7450;
/// VST 2.4 version constant.
pub const VST_VERSION: i32 = 2400;

pub const EFF_FLAGS_HAS_EDITOR: i32 = 1;
pub const EFF_FLAGS_CAN_REPLACING: i32 = 1 << 4;
pub const EFF_FLAGS_IS_SYNTH: i32 = 1 << 8;

/// Opcodes sent through `AEffect::dispatcher` (VST 2.4).
pub mod eff {
    pub const OPEN: i32 = 0;
    pub const CLOSE: i32 = 1;
    pub const SET_PROGRAM_NAME: i32 = 4;
    pub const GET_PROGRAM_NAME: i32 = 5;
    pub const GET_PARAM_LABEL: i32 = 6;
    pub const GET_PARAM_DISPLAY: i32 = 7;
    pub const GET_PARAM_NAME: i32 = 8;
    pub const SET_SAMPLE_RATE: i32 = 10;
    pub const SET_BLOCK_SIZE: i32 = 11;
    pub const MAINS_CHANGED: i32 = 12;
    pub const EDIT_GET_RECT: i32 = 13;
    pub const EDIT_OPEN: i32 = 14;
    pub const EDIT_CLOSE: i32 = 15;
    pub const GET_CHUNK: i32 = 23;
    pub const SET_CHUNK: i32 = 24;
    pub const PROCESS_EVENTS: i32 = 25;
    pub const GET_EFFECT_NAME: i32 = 45;
    pub const GET_VENDOR_STRING: i32 = 47;
    pub const GET_PRODUCT_STRING: i32 = 48;
    pub const GET_VENDOR_VERSION: i32 = 49;
    pub const GET_TAIL_SIZE: i32 = 52;
}

pub type Dispatcher =
    unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
pub type ProcessProc =
    unsafe extern "C" fn(*mut AEffect, *const *const f32, *mut *mut f32, i32);
pub type ProcessDoubleProc =
    unsafe extern "C" fn(*mut AEffect, *const *const f64, *mut *mut f64, i32);
pub type SetParameterProc = unsafe extern "C" fn(*mut AEffect, i32, f32);
pub type GetParameterProc = unsafe extern "C" fn(*mut AEffect, i32) -> f32;
pub type AudioMaster = unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
pub type PluginMain = unsafe extern "C" fn(AudioMaster) -> *mut AEffect;

/// Public VST 2.4 `AEffect` layout. Field offsets are checked in tests.
#[repr(C)]
pub struct AEffect {
    pub magic: i32,
    pub dispatcher: Option<Dispatcher>,
    pub process: Option<ProcessProc>,
    pub set_parameter: Option<SetParameterProc>,
    pub get_parameter: Option<GetParameterProc>,
    pub num_programs: i32,
    pub num_params: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub flags: i32,
    pub ptr1: isize,
    pub ptr2: isize,
    pub initial_delay: i32,
    pub real_qualities: i32,
    pub off_qualities: i32,
    pub io_ratio: f32,
    pub object: *mut c_void,
    pub user: *mut c_void,
    pub unique_id: i32,
    pub version: i32,
    pub process_replacing: Option<ProcessProc>,
    pub process_double_replacing: Option<ProcessDoubleProc>,
    pub future: [u8; 56],
}

// ------------------------------------------------------------- host callback

static HOST_SR: AtomicU32 = AtomicU32::new(48_000);
static HOST_BLOCK: AtomicU32 = AtomicU32::new(512);
static HOST_TEMPO_MILLI: AtomicU32 = AtomicU32::new(120_000);
static HOST_PLAYING: AtomicBool = AtomicBool::new(false);

mod audio_master_op {
    pub const VERSION: i32 = 1;
    pub const GET_TIME: i32 = 7;
    pub const TEMPO_AT: i32 = 11;
    pub const GET_SAMPLE_RATE: i32 = 16;
    pub const GET_BLOCK_SIZE: i32 = 17;
    pub const GET_CURRENT_PROCESS_LEVEL: i32 = 23;
    pub const GET_LANGUAGE: i32 = 32;
}

/// VST2's host callback has no user pointer, so the engine-wide transport facts
/// live in atomics. REDRAW runs one engine at 48 kHz/512, which matches these.
unsafe extern "C" fn audio_master(
    _effect: *mut AEffect,
    opcode: i32,
    _index: i32,
    _value: isize,
    _ptr: *mut c_void,
    _opt: f32,
) -> isize {
    match opcode {
        audio_master_op::VERSION => VST_VERSION as isize,
        audio_master_op::GET_SAMPLE_RATE => HOST_SR.load(Ordering::Relaxed) as isize,
        audio_master_op::GET_BLOCK_SIZE => HOST_BLOCK.load(Ordering::Relaxed) as isize,
        audio_master_op::TEMPO_AT => HOST_TEMPO_MILLI.load(Ordering::Relaxed) as isize / 1000,
        audio_master_op::GET_CURRENT_PROCESS_LEVEL => 2, // realtime
        audio_master_op::GET_LANGUAGE => 1,               // English
        audio_master_op::GET_TIME => 0,                    // no VstTimeInfo
        _ => 0,
    }
}

/// Update the transport facts exposed to hosted plugins. Called by the engine
/// when tempo or play state changes (control thread).
pub fn set_host_transport(tempo_bpm: f32, playing: bool) {
    HOST_TEMPO_MILLI.store(
        (tempo_bpm.clamp(20.0, 300.0) * 1000.0) as u32,
        Ordering::Relaxed,
    );
    HOST_PLAYING.store(playing, Ordering::Relaxed);
}

pub fn host_sample_rate() -> u32 {
    HOST_SR.load(Ordering::Relaxed)
}

pub fn host_block_size() -> u32 {
    HOST_BLOCK.load(Ordering::Relaxed)
}

pub fn host_playing() -> bool {
    HOST_PLAYING.load(Ordering::Relaxed)
}

// --------------------------------------------------------------------- errors

#[derive(Debug)]
pub enum VstError {
    Io(std::io::Error),
    Lib(libloading::Error),
    Json(serde_json::Error),
    Load(String),
    Unsupported(String),
}

impl fmt::Display for VstError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VstError::Io(e) => write!(f, "I/O error: {e}"),
            VstError::Lib(e) => write!(f, "library error: {e}"),
            VstError::Json(e) => write!(f, "cache error: {e}"),
            VstError::Load(msg) => write!(f, "load error: {msg}"),
            VstError::Unsupported(msg) => write!(f, "unsupported: {msg}"),
        }
    }
}

impl std::error::Error for VstError {}

impl From<std::io::Error> for VstError {
    fn from(value: std::io::Error) -> Self {
        VstError::Io(value)
    }
}

impl From<libloading::Error> for VstError {
    fn from(value: libloading::Error) -> Self {
        VstError::Lib(value)
    }
}

impl From<serde_json::Error> for VstError {
    fn from(value: serde_json::Error) -> Self {
        VstError::Json(value)
    }
}

// ---------------------------------------------------------------- plugin info

#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    pub name: String,
    pub vendor: String,
    pub product: String,
    pub path: String,
    pub version: i32,
    pub unique_id: i32,
    pub num_params: usize,
    pub num_inputs: usize,
    pub num_outputs: usize,
    pub is_synth: bool,
    pub has_editor: bool,
    pub can_replacing: bool,
    pub param_names: Vec<String>,
}

// ------------------------------------------------------------------ utilities

unsafe fn dispatch(
    effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) -> isize {
    let f = unsafe { (*effect).dispatcher };
    match f {
        Some(f) => unsafe { f(effect, opcode, index, value, ptr, opt) },
        None => 0,
    }
}

fn c_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

unsafe fn read_string(effect: *mut AEffect, opcode: i32, index: i32) -> String {
    let mut buf = [0u8; 256];
    unsafe {
        dispatch(
            effect,
            opcode,
            index,
            0,
            buf.as_mut_ptr() as *mut c_void,
            0.0,
        );
    }
    c_string(&buf)
}

// ------------------------------------------------------------------ VST2 host

/// A loaded VST2 effect instance. Construction follows the public host sequence:
/// open, set sample rate, set block size, resume. Drop pauses, closes and then
/// releases the library.
pub struct Vst2Plugin {
    library: libloading::Library,
    effect: *mut AEffect,
    info: PluginInfo,
    block_size: usize,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    active: bool,
    editor_open: bool,
}

// SAFETY: a hosted instance is owned by exactly one thread at a time. The
// engine moves effects between the control and audio threads only while audio
// is stopped (construction/removal), never concurrently.
unsafe impl Send for Vst2Plugin {}

impl Vst2Plugin {
    pub fn load(
        path: impl AsRef<Path>,
        sample_rate: f32,
        block_size: usize,
    ) -> Result<Self, VstError> {
        let path = path.as_ref();
        let sample_rate = sample_rate.clamp(8_000.0, 192_000.0);
        let block_size = block_size.clamp(64, 8192);
        HOST_SR.store(sample_rate as u32, Ordering::Relaxed);
        HOST_BLOCK.store(block_size as u32, Ordering::Relaxed);

        let library = unsafe { libloading::Library::new(path)? };
        let main = unsafe {
            library
                .get::<PluginMain>(b"VSTPluginMain\0")
                .or_else(|_| library.get::<PluginMain>(b"main\0"))?
        };
        let effect = unsafe { main(audio_master) };
        drop(main);
        if effect.is_null() {
            return Err(VstError::Load("VSTPluginMain returned null".into()));
        }
        let magic = unsafe { (*effect).magic };
        if magic != VST_MAGIC {
            return Err(VstError::Load(format!(
                "bad AEffect magic 0x{magic:08x}"
            )));
        }
        let dispatcher_missing = unsafe { (*effect).dispatcher.is_none() };
        if dispatcher_missing {
            return Err(VstError::Load("AEffect has no dispatcher".into()));
        }
        let num_outputs = unsafe { (*effect).num_outputs }.max(0) as usize;
        if num_outputs == 0 {
            return Err(VstError::Unsupported("plug-in has no audio outputs".into()));
        }
        let can_replacing = unsafe {
            ((*effect).flags & EFF_FLAGS_CAN_REPLACING) != 0
                && (*effect).process_replacing.is_some()
        };
        if !can_replacing {
            return Err(VstError::Unsupported(
                "plug-in does not implement processReplacing".into(),
            ));
        }

        unsafe {
            dispatch(effect, eff::OPEN, 0, 0, std::ptr::null_mut(), 0.0);
            dispatch(
                effect,
                eff::SET_SAMPLE_RATE,
                0,
                0,
                std::ptr::null_mut(),
                sample_rate,
            );
            dispatch(
                effect,
                eff::SET_BLOCK_SIZE,
                0,
                block_size as isize,
                std::ptr::null_mut(),
                0.0,
            );
            dispatch(
                effect,
                eff::MAINS_CHANGED,
                0,
                1,
                std::ptr::null_mut(),
                0.0,
            );
        }

        let num_params = unsafe { (*effect).num_params }.max(0) as usize;
        let mut param_names = Vec::with_capacity(num_params);
        for index in 0..num_params {
            param_names.push(unsafe { read_string(effect, eff::GET_PARAM_NAME, index as i32) });
        }
        let mut name = unsafe { read_string(effect, eff::GET_EFFECT_NAME, 0) };
        if name.is_empty() {
            name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("VST2")
                .to_string();
        }
        let vendor = unsafe { read_string(effect, eff::GET_VENDOR_STRING, 0) };
        let product = unsafe { read_string(effect, eff::GET_PRODUCT_STRING, 0) };
        let vendor_version =
            unsafe { dispatch(effect, eff::GET_VENDOR_VERSION, 0, 0, std::ptr::null_mut(), 0.0) }
                as i32;
        let (unique_id, struct_version, flags, num_inputs) = unsafe {
            (
                (*effect).unique_id,
                (*effect).version,
                (*effect).flags,
                (*effect).num_inputs.max(0) as usize,
            )
        };
        let info = PluginInfo {
            name,
            vendor,
            product,
            path: path.display().to_string(),
            version: if vendor_version != 0 {
                vendor_version
            } else {
                struct_version
            },
            unique_id,
            num_params,
            num_inputs,
            num_outputs,
            is_synth: flags & EFF_FLAGS_IS_SYNTH != 0,
            has_editor: flags & EFF_FLAGS_HAS_EDITOR != 0,
            can_replacing,
            param_names,
        };

        Ok(Self {
            library,
            effect,
            info,
            block_size,
            in_l: vec![0.0; block_size],
            in_r: vec![0.0; block_size],
            out_l: vec![0.0; block_size],
            out_r: vec![0.0; block_size],
            active: true,
            editor_open: false,
        })
    }

    pub fn info(&self) -> &PluginInfo {
        &self.info
    }

    pub fn param_name(&self, index: usize) -> &str {
        self.info
            .param_names
            .get(index)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn param_display(&self, index: usize) -> String {
        if index >= self.info.num_params {
            return String::new();
        }
        let mut buf = [0u8; 128];
        unsafe {
            dispatch(
                self.effect,
                eff::GET_PARAM_DISPLAY,
                index as i32,
                0,
                buf.as_mut_ptr() as *mut c_void,
                0.0,
            );
        }
        c_string(&buf)
    }

    pub fn param_label(&self, index: usize) -> String {
        if index >= self.info.num_params {
            return String::new();
        }
        let mut buf = [0u8; 64];
        unsafe {
            dispatch(
                self.effect,
                eff::GET_PARAM_LABEL,
                index as i32,
                0,
                buf.as_mut_ptr() as *mut c_void,
                0.0,
            );
        }
        c_string(&buf)
    }

    /// Set a normalized 0..1 parameter through the plug-in's own setter.
    pub fn set_param_normalized(&mut self, index: usize, value: f32) {
        if index >= self.info.num_params {
            return;
        }
        if let Some(set) = unsafe { (*self.effect).set_parameter } {
            unsafe { set(self.effect, index as i32, value.clamp(0.0, 1.0)) };
        }
    }

    pub fn param_normalized(&self, index: usize) -> f32 {
        if index >= self.info.num_params {
            return 0.0;
        }
        match unsafe { (*self.effect).get_parameter } {
            Some(get) => unsafe { get(self.effect, index as i32) }.clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// Process planar stereo in place. Chunks at the prepared block size; no
    /// allocation, locks or I/O. Mono outputs are duplicated to both channels.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let proc = unsafe { (*self.effect).process_replacing };
        let Some(proc) = proc else {
            return;
        };
        let mut offset = 0usize;
        while offset < frames {
            let n = (frames - offset).min(self.block_size);
            for i in 0..n {
                let (l, r) = (left[offset + i], right[offset + i]);
                if self.info.num_inputs >= 2 {
                    self.in_l[i] = l;
                    self.in_r[i] = r;
                } else {
                    self.in_l[i] = (l + r) * 0.5;
                    self.in_r[i] = self.in_l[i];
                }
                self.out_l[i] = 0.0;
                self.out_r[i] = 0.0;
            }
            let inputs: [*const f32; 2] = [self.in_l.as_ptr(), self.in_r.as_ptr()];
            let mut outputs: [*mut f32; 2] = [self.out_l.as_mut_ptr(), self.out_r.as_mut_ptr()];
            unsafe {
                proc(
                    self.effect,
                    inputs.as_ptr(),
                    outputs.as_mut_ptr(),
                    n as i32,
                );
            }
            if self.info.num_outputs >= 2 {
                left[offset..offset + n].copy_from_slice(&self.out_l[..n]);
                right[offset..offset + n].copy_from_slice(&self.out_r[..n]);
            } else {
                left[offset..offset + n].copy_from_slice(&self.out_l[..n]);
                right[offset..offset + n].copy_from_slice(&self.out_l[..n]);
            }
            offset += n;
        }
    }

    /// Raw state chunk (bank). Empty when the plug-in exposes no chunk. The
    /// pointer returned by `effGetChunk` is not freed: freeing a plug-in
    /// allocation through the host CRT is unsafe across runtimes (documented).
    pub fn get_state(&self) -> Vec<u8> {
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let size = unsafe {
            dispatch(
                self.effect,
                eff::GET_CHUNK,
                0,
                0,
                &mut ptr as *mut *mut c_void as *mut c_void,
                0.0,
            )
        };
        if size <= 0 || ptr.is_null() {
            return Vec::new();
        }
        unsafe { std::slice::from_raw_parts(ptr as *const u8, size as usize).to_vec() }
    }

    /// Restore a bank chunk. Returns false when the plug-in rejects it.
    pub fn set_state(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        let result = unsafe {
            dispatch(
                self.effect,
                eff::SET_CHUNK,
                0,
                bytes.len() as isize,
                bytes.as_ptr() as *mut c_void,
                0.0,
            )
        };
        result == 0
    }

    /// Open the plug-in editor inside `parent_hwnd` (Win32). Returns false when
    /// the plug-in has no editor or refuses.
    pub fn open_editor(&mut self, parent_hwnd: isize) -> bool {
        if !self.info.has_editor || self.editor_open {
            return false;
        }
        let result = unsafe {
            dispatch(
                self.effect,
                eff::EDIT_OPEN,
                0,
                0,
                parent_hwnd as *mut c_void,
                0.0,
            )
        };
        if result == 0 {
            self.editor_open = true;
        }
        self.editor_open
    }

    pub fn close_editor(&mut self) {
        if self.editor_open {
            unsafe {
                dispatch(
                    self.effect,
                    eff::EDIT_CLOSE,
                    0,
                    0,
                    std::ptr::null_mut(),
                    0.0,
                );
            }
            self.editor_open = false;
        }
    }
}

impl Drop for Vst2Plugin {
    fn drop(&mut self) {
        self.close_editor();
        if self.active {
            unsafe {
                dispatch(
                    self.effect,
                    eff::MAINS_CHANGED,
                    0,
                    0,
                    std::ptr::null_mut(),
                    0.0,
                );
            }
            self.active = false;
        }
        unsafe {
            dispatch(
                self.effect,
                eff::CLOSE,
                0,
                0,
                std::ptr::null_mut(),
                0.0,
            );
        }
        // The library is released after the plug-in's effClose above.
        let _ = &self.library;
    }
}

// ---------------------------------------------------------------- effect glue

/// `Effect` adapter so hosted VST2 effects share the mixer path. Parameter specs
/// are fixed 0..1 normalized ranges; names are leaked once at construction
/// (plug-in metadata is stable for the instance lifetime).
pub struct Vst2Effect {
    plugin: Vst2Plugin,
    name: &'static str,
    specs: &'static [ParamSpec],
    bypassed: bool,
}

impl Vst2Effect {
    pub fn new(plugin: Vst2Plugin) -> Self {
        let name: &'static str = Box::leak(plugin.info().name.clone().into_boxed_str());
        let mut specs = Vec::with_capacity(plugin.info().num_params);
        for index in 0..plugin.info().num_params {
            let pname: &'static str =
                Box::leak(plugin.param_name(index).to_string().into_boxed_str());
            let default = plugin.param_normalized(index);
            specs.push(ParamSpec::new(pname, 0.0, 1.0, default, ""));
        }
        let specs: &'static [ParamSpec] = Box::leak(specs.into_boxed_slice());
        Self {
            plugin,
            name,
            specs,
            bypassed: false,
        }
    }

    pub fn info(&self) -> &PluginInfo {
        self.plugin.info()
    }

    pub fn get_state(&self) -> Vec<u8> {
        self.plugin.get_state()
    }

    pub fn set_state(&mut self, bytes: &[u8]) -> bool {
        self.plugin.set_state(bytes)
    }
}

impl Effect for Vst2Effect {
    fn effect_type(&self) -> &'static str {
        self.name
    }

    fn reset(&mut self) {}

    fn process(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        if self.bypassed {
            return;
        }
        self.plugin.process(left, right);
    }

    fn params(&self) -> &'static [ParamSpec] {
        self.specs
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.plugin.set_param_normalized(index, value);
    }

    fn get_param(&self, index: usize) -> f32 {
        self.plugin.param_normalized(index)
    }

    fn set_bypass(&mut self, bypass: bool) {
        self.bypassed = bypass;
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn controller_value(&self) -> f32 {
        0.0
    }
}

// -------------------------------------------------------------------- host

/// Control-thread factory bound to an engine format.
#[derive(Clone, Debug)]
pub struct VstHost {
    sample_rate: f32,
    block_size: usize,
}

impl VstHost {
    pub fn new(sample_rate: f32, block_size: usize) -> Self {
        Self {
            sample_rate: sample_rate.clamp(8_000.0, 192_000.0),
            block_size: block_size.clamp(64, 8192),
        }
    }

    pub fn load_effect(&self, path: impl AsRef<Path>) -> Result<Vst2Effect, VstError> {
        let plugin = Vst2Plugin::load(path, self.sample_rate, self.block_size)?;
        Ok(Vst2Effect::new(plugin))
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }
}

// ---------------------------------------------------------------- discovery

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    Vst2,
    Vst3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginKind {
    Effect,
    Instrument,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub path: String,
    pub name: String,
    pub format: PluginFormat,
    pub kind: PluginKind,
}

/// Recursively scan roots for `.dll` and `.vst3` modules. VST3 bundles are
/// treated as single entries instead of being descended into.
pub fn discover(roots: &[PathBuf]) -> Vec<PluginEntry> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        walk_plugins(root, 0, &mut out, &mut seen);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn walk_plugins(
    dir: &Path,
    depth: usize,
    out: &mut Vec<PluginEntry>,
    seen: &mut std::collections::HashSet<PathBuf>,
) {
    if depth > 8 || !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let lower = name.to_ascii_lowercase();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if lower.ends_with(".vst3") {
                if seen.insert(path.clone()) {
                    out.push(PluginEntry {
                        path: path.display().to_string(),
                        name: path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("VST3")
                            .to_string(),
                        format: PluginFormat::Vst3,
                        kind: PluginKind::Unknown,
                    });
                }
            } else {
                walk_plugins(&path, depth + 1, out, seen);
            }
        } else if meta.is_file() {
            let format = if lower.ends_with(".dll") {
                PluginFormat::Vst2
            } else if lower.ends_with(".vst3") {
                PluginFormat::Vst3
            } else {
                continue;
            };
            if seen.insert(path.clone()) {
                out.push(PluginEntry {
                    path: path.display().to_string(),
                    name: path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("plugin")
                        .to_string(),
                    format,
                    kind: PluginKind::Unknown,
                });
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    plugins: Vec<PluginEntry>,
}

/// Persist scan results (tolerant: missing files load as an empty cache).
pub fn save_cache(path: impl AsRef<Path>, plugins: &[PluginEntry]) -> Result<(), VstError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = CacheFile {
        version: 1,
        plugins: plugins.to_vec(),
    };
    std::fs::write(path, serde_json::to_vec_pretty(&file)?)?;
    Ok(())
}

pub fn load_cache(path: impl AsRef<Path>) -> Result<Vec<PluginEntry>, VstError> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path)?;
    let file: CacheFile = serde_json::from_slice(&bytes)?;
    if file.version != 1 {
        return Ok(Vec::new());
    }
    Ok(file.plugins)
}

// -------------------------------------------------------------------- VST3

/// Locate the loadable module inside a VST3 bundle (or return a file path).
pub fn vst3_module(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let standard = path.join("Contents").join("x86_64-win");
    if let Some(found) = find_module_in(&standard, 0) {
        return Some(found);
    }
    find_module_in(path, 0)
}

fn find_module_in(dir: &Path, depth: usize) -> Option<PathBuf> {
    if depth > 6 || !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let lower = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if path.is_file() && lower.ends_with(".vst3") {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_module_in(&path, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vst3Probe {
    pub module: String,
    pub factory_present: bool,
}

/// Probe a VST3 bundle for the `GetPluginFactory` export without instantiating
/// it. Full COM hosting is deferred (D-017); discovery still reports the module
/// so the UI can list it.
pub fn probe_vst3(path: impl AsRef<Path>) -> Result<Vst3Probe, VstError> {
    let module = vst3_module(path)
        .ok_or_else(|| VstError::Load("no VST3 module found in bundle".into()))?;
    let library = unsafe { libloading::Library::new(&module)? };
    type GetFactory = unsafe extern "C" fn() -> *mut c_void;
    let present = unsafe { library.get::<GetFactory>(b"GetPluginFactory\0").is_ok() };
    Ok(Vst3Probe {
        module: module.display().to_string(),
        factory_present: present,
    })
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn vst2_abi_layout() {
        #[cfg(target_pointer_width = "64")]
        {
            use std::mem::{offset_of, size_of};
            assert_eq!(size_of::<AEffect>(), 192);
            assert_eq!(offset_of!(AEffect, magic), 0);
            assert_eq!(offset_of!(AEffect, dispatcher), 8);
            assert_eq!(offset_of!(AEffect, set_parameter), 24);
            assert_eq!(offset_of!(AEffect, num_params), 44);
            assert_eq!(offset_of!(AEffect, flags), 56);
            assert_eq!(offset_of!(AEffect, object), 96);
            assert_eq!(offset_of!(AEffect, unique_id), 112);
            assert_eq!(offset_of!(AEffect, process_replacing), 120);
            assert_eq!(offset_of!(AEffect, future), 136);
        }
    }

    #[test]
    fn discovery_and_cache_round_trip() {
        let root = tmp_dir("vst_discovery");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("alpha.dll"), b"not a dll").unwrap();
        std::fs::write(root.join("sub").join("beta.DLL"), b"not a dll").unwrap();
        std::fs::create_dir_all(root.join("gamma.vst3")).unwrap();
        std::fs::write(root.join("readme.txt"), b"ignore").unwrap();

        let entries = discover(&[root.clone()]);
        assert_eq!(entries.len(), 3, "entries: {entries:?}");
        assert!(entries.iter().any(|e| e.name == "alpha" && e.format == PluginFormat::Vst2));
        assert!(entries.iter().any(|e| e.name == "beta" && e.format == PluginFormat::Vst2));
        assert!(
            entries
                .iter()
                .any(|e| e.name == "gamma" && e.format == PluginFormat::Vst3)
        );

        let cache = root.join("plugins.json");
        save_cache(&cache, &entries).expect("save cache");
        let loaded = load_cache(&cache).expect("load cache");
        assert_eq!(loaded, entries);
        assert!(load_cache(root.join("missing.json")).expect("missing").is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn vst3_module_resolution() {
        let root = tmp_dir("vst3_bundle");
        let bin = root.join("Contents").join("x86_64-win");
        std::fs::create_dir_all(&bin).unwrap();
        let module = bin.join("demo.vst3");
        std::fs::write(&module, b"not a real module").unwrap();
        let found = vst3_module(&root).expect("module path");
        assert_eq!(found, module);
        assert!(vst3_module(root.join("nothing-here")).is_none());
        // A fake module is found but cannot be loaded; the probe reports the error.
        assert!(probe_vst3(&root).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn hosted_plugin_parameter_normalization() {
        // Pure helper behavior that does not need a DLL: ParamSpec normalization
        // is 0..1 for hosted params.
        let spec = ParamSpec::new("Gain", 0.0, 1.0, 0.5, "");
        assert!((spec.denormalize(0.75) - 0.75).abs() < 1e-6);
        assert!((spec.normalize(0.25) - 0.25).abs() < 1e-6);
    }

    const MOCK_VST2_SOURCE: &str = r##"
#![allow(unsafe_code)]
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

type Dispatcher = unsafe extern "C" fn(*mut AEffect, i32, i32, isize, *mut c_void, f32) -> isize;
type Process = unsafe extern "C" fn(*mut AEffect, *const *const f32, *mut *mut f32, i32);
type SetParam = unsafe extern "C" fn(*mut AEffect, i32, f32);
type GetParam = unsafe extern "C" fn(*mut AEffect, i32) -> f32;
type ProcessDouble = unsafe extern "C" fn(*mut AEffect, *const *const f64, *mut *mut f64, i32);

#[repr(C)]
pub struct AEffect {
    magic: i32,
    dispatcher: Option<Dispatcher>,
    process: Option<Process>,
    set_parameter: Option<SetParam>,
    get_parameter: Option<GetParam>,
    num_programs: i32,
    num_params: i32,
    num_inputs: i32,
    num_outputs: i32,
    flags: i32,
    ptr1: isize,
    ptr2: isize,
    initial_delay: i32,
    real_qualities: i32,
    off_qualities: i32,
    io_ratio: f32,
    object: *mut c_void,
    user: *mut c_void,
    unique_id: i32,
    version: i32,
    process_replacing: Option<Process>,
    process_double_replacing: Option<ProcessDouble>,
    future: [u8; 56],
}

static PARAM: AtomicU32 = AtomicU32::new(0u32);
static CHUNK: [u8; 10] = *b"mock-state";

unsafe fn write_str(dst: *mut c_void, s: &str) {
    if dst.is_null() { return; }
    let out = dst as *mut u8;
    ptr::copy_nonoverlapping(s.as_ptr(), out, s.len());
    *out.add(s.len()) = 0;
}

unsafe extern "C" fn dispatcher(
    _effect: *mut AEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr_: *mut c_void,
    _opt: f32,
) -> isize {
    match opcode {
        0 | 1 => 0,
        8 => { if index == 0 { unsafe { write_str(ptr_, "Mock Gain") }; } 0 }
        10 | 11 | 12 => 0,
        23 => {
            if !ptr_.is_null() {
                let out = ptr_ as *mut *mut u8;
                unsafe { *out = CHUNK.as_ptr() as *mut u8 };
            }
            CHUNK.len() as isize
        }
        24 => {
            if value == CHUNK.len() as isize && !ptr_.is_null() {
                let bytes = unsafe { std::slice::from_raw_parts(ptr_ as *const u8, value as usize) };
                if bytes == &CHUNK[..] {
                    PARAM.store(0.25f32.to_bits(), Ordering::Relaxed);
                }
            }
            0
        }
        45 => { unsafe { write_str(ptr_, "Mock Plugin") }; 0 }
        47 => { unsafe { write_str(ptr_, "REDRAW Tests") }; 0 }
        48 => { unsafe { write_str(ptr_, "Mock") }; 0 }
        49 => 123,
        _ => 0,
    }
}

unsafe extern "C" fn set_param(_effect: *mut AEffect, index: i32, value: f32) {
    if index == 0 {
        PARAM.store(value.to_bits(), Ordering::Relaxed);
    }
}

unsafe extern "C" fn get_param(_effect: *mut AEffect, index: i32) -> f32 {
    if index == 0 { f32::from_bits(PARAM.load(Ordering::Relaxed)) } else { 0.0 }
}

unsafe extern "C" fn process(
    effect: *mut AEffect,
    inputs: *const *const f32,
    outputs: *mut *mut f32,
    frames: i32,
) {
    let gain = 0.5 + 0.5 * unsafe { get_param(effect, 0) };
    let out_l = unsafe { *outputs };
    let out_r = unsafe { *outputs.add(1) };
    let in_l = if inputs.is_null() { ptr::null() } else { unsafe { *inputs } };
    let in_r = if inputs.is_null() { ptr::null() } else { unsafe { *inputs.add(1) } };
    for i in 0..frames.max(0) as usize {
        let l = if in_l.is_null() { 0.0 } else { unsafe { *in_l.add(i) } };
        let r = if in_r.is_null() { 0.0 } else { unsafe { *in_r.add(i) } };
        unsafe {
            *out_l.add(i) = l * gain;
            *out_r.add(i) = r * gain;
        }
    }
}

#[no_mangle]
pub extern "C" fn VSTPluginMain(_host: *mut c_void) -> *mut AEffect {
    let effect = AEffect {
        magic: 0x56737450,
        dispatcher: Some(dispatcher),
        process: None,
        set_parameter: Some(set_param),
        get_parameter: Some(get_param),
        num_programs: 1,
        num_params: 1,
        num_inputs: 2,
        num_outputs: 2,
        flags: 1 << 4,
        ptr1: 0,
        ptr2: 0,
        initial_delay: 0,
        real_qualities: 0,
        off_qualities: 0,
        io_ratio: 1.0,
        object: ptr::null_mut(),
        user: ptr::null_mut(),
        unique_id: 0x4D4F434B,
        version: 100,
        process_replacing: Some(process),
        process_double_replacing: None,
        future: [0; 56],
    };
    Box::into_raw(Box::new(effect))
}
"##;

    #[test]
    fn vst2_mock_loads_processes_and_restores_state() {
        let dir = tmp_dir("vst2_mock");
        let source = dir.join("mock_plugin.rs");
        let dll = dir.join("mock_plugin.dll");
        let mut file = std::fs::File::create(&source).expect("write mock source");
        file.write_all(MOCK_VST2_SOURCE.as_bytes())
            .expect("write mock source");
        drop(file);
        let status = std::process::Command::new("rustc")
            .arg("--crate-type")
            .arg("cdylib")
            .arg("--edition")
            .arg("2021")
            .arg("-O")
            .arg(&source)
            .arg("-o")
            .arg(&dll)
            .status()
            .expect("run rustc");
        assert!(status.success(), "mock plug-in did not compile");

        let host = VstHost::new(48_000.0, 512);
        let mut effect = host.load_effect(&dll).expect("load mock plug-in");
        assert_eq!(effect.info().name, "Mock Plugin");
        assert_eq!(effect.info().vendor, "REDRAW Tests");
        assert_eq!(effect.info().num_params, 1);
        assert_eq!(effect.get_param_name(0), "Mock Gain");
        assert!(effect.info().can_replacing);
        assert!(!effect.info().has_editor);

        let mut left = vec![1.0f32; 128];
        let mut right = vec![0.5f32; 128];
        effect.process(&mut left, &mut right, 48_000.0, 120.0);
        assert!((left[0] - 0.5).abs() < 1e-6, "default gain {}", left[0]);
        assert!((right[0] - 0.25).abs() < 1e-6);

        effect.set_param(0, 1.0);
        let mut left = vec![1.0f32; 128];
        let mut right = vec![0.5f32; 128];
        effect.process(&mut left, &mut right, 48_000.0, 120.0);
        assert!((left[0] - 1.0).abs() < 1e-6);
        assert!((right[0] - 0.5).abs() < 1e-6);

        assert_eq!(effect.get_state(), b"mock-state".to_vec());
        assert!(effect.set_state(b"mock-state"));
        assert!((effect.get_param(0) - 0.25).abs() < 1e-6);

        // Bypass is exact passthrough.
        effect.set_bypass(true);
        let mut dry_l = vec![0.3f32; 32];
        let mut dry_r = vec![-0.2f32; 32];
        let snapshot_l = dry_l.clone();
        effect.process(&mut dry_l, &mut dry_r, 48_000.0, 120.0);
        assert_eq!(dry_l, snapshot_l);
        assert_eq!(dry_r, vec![-0.2f32; 32]);

        drop(effect);
        std::fs::remove_dir_all(&dir).ok();
    }
}
