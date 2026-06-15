//! Minimal NvAPI FFI: init, GPU enum, I2C read/write, thermal read.
//! Hashes and struct layouts reverse-engineered from ThPanel.exe.

use libloading::{Library, Symbol};
use std::ffi::c_void;
use std::ptr;

// NvAPI function hashes (from ThPanel.exe).
const HASH_INITIALIZE: u32 = 0x0150_E828;
const HASH_ENUM_GPUS: u32 = 0xE5AC_921F;
const HASH_I2C_WRITE_EX: u32 = 0x283A_C65A;
const HASH_I2C_READ_EX: u32 = 0x4D7B_0709;
const HASH_GET_THERMAL: u32 = 0xE364_0A56;

const I2C_INFO_VERSION: u32 = 0x0003_0040; // NV_I2C_INFO_V3 (64 bytes)
const THERMAL_VERSION: u32 = 0x0002_0044; // NV_GPU_THERMAL_SETTINGS_V2

type GpuHandle = *mut c_void;

// NV_I2C_INFO_V3, exact 64-byte layout used by ThPanel.
#[repr(C)]
struct NvI2cInfoV3 {
    version: u32,         // 0
    display_mask: u32,    // 4
    b_is_ddc: u8,         // 8
    dev_addr: u8,         // 9
    _pad0: [u8; 6],       // 10
    reg_addr: *mut u8,    // 16
    reg_size: u32,        // 24
    _pad1: u32,           // 28
    data: *mut u8,        // 32
    cb_size: u32,         // 40
    i2c_speed: u32,       // 44
    speed_khz: u32,       // 48
    port_id: u8,          // 52
    _pad2: [u8; 3],       // 53
    is_port_set: u32,     // 56
    _pad3: u32,           // 60
}

type QiFn = unsafe extern "C" fn(u32) -> *const c_void;
type InitFn = unsafe extern "C" fn() -> i32;
type EnumFn = unsafe extern "C" fn(*mut GpuHandle, *mut u32) -> i32;
type I2cFn = unsafe extern "C" fn(GpuHandle, *mut NvI2cInfoV3, *mut u32) -> i32;
type ThermalFn = unsafe extern "C" fn(GpuHandle, u32, *mut u8) -> i32;

pub struct NvApi {
    _lib: Library,
    qi: QiFn,
    gpus: Vec<GpuHandle>,
}

impl NvApi {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let lib = Library::new("nvapi64.dll")
                .map_err(|e| format!("load nvapi64.dll: {e}"))?;
            let qi_sym: Symbol<QiFn> = lib
                .get(b"nvapi_QueryInterface\0")
                .map_err(|e| format!("nvapi_QueryInterface: {e}"))?;
            let qi = *qi_sym;

            let init: InitFn = std::mem::transmute(resolve(qi, HASH_INITIALIZE)?);
            if init() != 0 {
                return Err("NvAPI_Initialize failed".into());
            }

            let mut me = NvApi { _lib: lib, qi, gpus: Vec::new() };
            me.gpus = me.enum_gpus()?;
            Ok(me)
        }
    }

    unsafe fn enum_gpus(&self) -> Result<Vec<GpuHandle>, String> {
        let f: EnumFn = std::mem::transmute(resolve(self.qi, HASH_ENUM_GPUS)?);
        let mut arr: [GpuHandle; 64] = [ptr::null_mut(); 64];
        let mut count: u32 = 0;
        if f(arr.as_mut_ptr(), &mut count) != 0 {
            return Err("NvAPI_EnumPhysicalGPUs failed".into());
        }
        Ok(arr[..count as usize].to_vec())
    }

    pub fn gpu_count(&self) -> usize {
        self.gpus.len()
    }

    fn handle(&self, idx: usize) -> Result<GpuHandle, String> {
        self.gpus
            .get(idx)
            .copied()
            .ok_or_else(|| format!("GPU index {idx} out of range (have {})", self.gpus.len()))
    }

    pub fn i2c_write(
        &self,
        gpu: usize,
        dev: u8,
        reg: u8,
        data: &[u8],
        port: u8,
    ) -> Result<(), String> {
        let h = self.handle(gpu)?;
        let mut reg_b = [reg];
        let mut buf = data.to_vec();
        let mut info = make_info(dev, &mut reg_b, &mut buf, port);
        let mut extra: u32 = 1;
        unsafe {
            let f: I2cFn = std::mem::transmute(resolve(self.qi, HASH_I2C_WRITE_EX)?);
            let r = f(h, &mut info, &mut extra);
            if r != 0 {
                return Err(format!("I2CWriteEx status {r}"));
            }
        }
        Ok(())
    }

    pub fn i2c_read(
        &self,
        gpu: usize,
        dev: u8,
        reg: u8,
        len: usize,
        port: u8,
    ) -> Result<Vec<u8>, String> {
        let h = self.handle(gpu)?;
        let mut reg_b = [reg];
        let mut buf = vec![0u8; len];
        let mut info = make_info(dev, &mut reg_b, &mut buf, port);
        let mut extra: u32 = 1;
        unsafe {
            let f: I2cFn = std::mem::transmute(resolve(self.qi, HASH_I2C_READ_EX)?);
            let r = f(h, &mut info, &mut extra);
            if r != 0 {
                return Err(format!("I2CReadEx status {r}"));
            }
        }
        Ok(buf)
    }

    /// GPU core temperature in degrees C (sensor 0).
    pub fn gpu_temp(&self, gpu: usize) -> Result<i32, String> {
        let h = self.handle(gpu)?;
        let mut s = [0u8; 84];
        s[0..4].copy_from_slice(&THERMAL_VERSION.to_le_bytes());
        unsafe {
            let f: ThermalFn = std::mem::transmute(resolve(self.qi, HASH_GET_THERMAL)?);
            // arg 15 = all sensors; struct holds results.
            if f(h, 15, s.as_mut_ptr()) != 0 {
                return Err("GetThermalSettings failed".into());
            }
        }
        // sensor0.currentTemp at byte offset 20.
        let t = i32::from_le_bytes([s[20], s[21], s[22], s[23]]);
        Ok(t)
    }
}

fn make_info(dev: u8, reg_b: &mut [u8; 1], data: &mut [u8], port: u8) -> NvI2cInfoV3 {
    NvI2cInfoV3 {
        version: I2C_INFO_VERSION,
        display_mask: 0,
        b_is_ddc: 0,
        dev_addr: dev,
        _pad0: [0; 6],
        reg_addr: reg_b.as_mut_ptr(),
        reg_size: 1,
        _pad1: 0,
        data: data.as_mut_ptr(),
        cb_size: data.len() as u32,
        i2c_speed: 0xFFFF,
        speed_khz: 0,
        port_id: port,
        _pad2: [0; 3],
        is_port_set: if port != 0 { 1 } else { 0 },
        _pad3: 0,
    }
}

unsafe fn resolve(qi: QiFn, hash: u32) -> Result<*const c_void, String> {
    let p = qi(hash);
    if p.is_null() {
        Err(format!("NvAPI hash {hash:#010x} not found"))
    } else {
        Ok(p)
    }
}
