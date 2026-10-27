//! Windows GPU source. DXGI names the adapters and reports their memory sizes; the
//! "GPU Engine" / "GPU Adapter Memory" performance counters give live usage. That is
//! the same data Windows Task Manager shows, so it works for every GPU vendor.
//!
//! Raw FFI keeps this free of the `windows` crate; only stable C / COM ABIs are used.

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

use super::gpu::{adapter_utilisation, pdh_luid, GpuReading};

type HRESULT = i32;
type PDH_STATUS = u32;
type HANDLE = *mut c_void;

const ERROR_SUCCESS: PDH_STATUS = 0;
const PDH_MORE_DATA: PDH_STATUS = 0x8000_07D2;
const PDH_CSTATUS_VALID_DATA: u32 = 0x0;
const PDH_CSTATUS_NEW_DATA: u32 = 0x1;
const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
const PDH_FMT_LARGE: u32 = 0x0000_0400;
const PDH_FMT_NOCAP100: u32 = 0x0000_8000;

/// Integrated GPUs report a token amount of dedicated memory; for those the shared
/// pool is the number that actually moves.
const DEDICATED_MIN: u64 = 512 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct PDH_FMT_COUNTERVALUE {
    CStatus: u32,
    /// The C union (double / LONGLONG / pointers); read per the requested format.
    bits: u64,
}

#[repr(C)]
struct PDH_FMT_COUNTERVALUE_ITEM_W {
    szName: *mut u16,
    FmtValue: PDH_FMT_COUNTERVALUE,
}

#[link(name = "pdh")]
extern "system" {
    fn PdhOpenQueryW(szDataSource: *const u16, dwUserData: usize, phQuery: *mut HANDLE) -> PDH_STATUS;
    fn PdhAddEnglishCounterW(
        hQuery: HANDLE,
        szFullCounterPath: *const u16,
        dwUserData: usize,
        phCounter: *mut HANDLE,
    ) -> PDH_STATUS;
    fn PdhCollectQueryData(hQuery: HANDLE) -> PDH_STATUS;
    fn PdhGetFormattedCounterArrayW(
        hCounter: HANDLE,
        dwFormat: u32,
        lpdwBufferSize: *mut u32,
        lpdwItemCount: *mut u32,
        ItemBuffer: *mut PDH_FMT_COUNTERVALUE_ITEM_W,
    ) -> PDH_STATUS;
    fn PdhCloseQuery(hQuery: HANDLE) -> PDH_STATUS;
}

#[repr(C)]
struct GUID {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

/// 770aae78-f26f-4dba-a829-253c83d1b387
const IID_IDXGI_FACTORY1: GUID = GUID {
    data1: 0x770a_ae78,
    data2: 0xf26f,
    data3: 0x4dba,
    data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
};

#[repr(C)]
#[derive(Clone, Copy)]
struct LUID {
    LowPart: u32,
    HighPart: i32,
}

#[repr(C)]
struct DXGI_ADAPTER_DESC1 {
    Description: [u16; 128],
    VendorId: u32,
    DeviceId: u32,
    SubSysId: u32,
    Revision: u32,
    DedicatedVideoMemory: usize,
    DedicatedSystemMemory: usize,
    SharedSystemMemory: usize,
    AdapterLuid: LUID,
    Flags: u32,
}

const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;

type ReleaseFn = unsafe extern "system" fn(this: *mut c_void) -> u32;

/// IUnknown → IDXGIObject → IDXGIFactory → IDXGIFactory1, in declaration order.
/// Slots we never call are plain pointer-sized placeholders.
#[repr(C)]
struct IDXGIFactory1Vtbl {
    QueryInterface: usize,
    AddRef: usize,
    Release: ReleaseFn,
    SetPrivateData: usize,
    SetPrivateDataInterface: usize,
    GetPrivateData: usize,
    GetParent: usize,
    EnumAdapters: usize,
    MakeWindowAssociation: usize,
    GetWindowAssociation: usize,
    CreateSwapChain: usize,
    CreateSoftwareAdapter: usize,
    EnumAdapters1: unsafe extern "system" fn(this: *mut c_void, adapter: u32, out: *mut *mut c_void) -> HRESULT,
    IsCurrent: usize,
}

/// IUnknown → IDXGIObject → IDXGIAdapter → IDXGIAdapter1.
#[repr(C)]
struct IDXGIAdapter1Vtbl {
    QueryInterface: usize,
    AddRef: usize,
    Release: ReleaseFn,
    SetPrivateData: usize,
    SetPrivateDataInterface: usize,
    GetPrivateData: usize,
    GetParent: usize,
    EnumOutputs: usize,
    GetDesc: usize,
    CheckInterfaceSupport: usize,
    GetDesc1: unsafe extern "system" fn(this: *mut c_void, desc: *mut DXGI_ADAPTER_DESC1) -> HRESULT,
}

#[link(name = "dxgi")]
extern "system" {
    fn CreateDXGIFactory1(riid: *const GUID, ppFactory: *mut *mut c_void) -> HRESULT;
}

struct Adapter {
    luid: String,
    name: String,
    vendor: u32,
    dedicated: u64,
    shared: u64,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Read a NUL-terminated UTF-16 string that PDH wrote into our buffer.
unsafe fn wide_ptr_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while len < 4096 && *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

fn dxgi_adapters() -> Vec<Adapter> {
    let mut out = Vec::new();
    unsafe {
        let mut factory: *mut c_void = ptr::null_mut();
        if CreateDXGIFactory1(&IID_IDXGI_FACTORY1, &mut factory) < 0 || factory.is_null() {
            return out;
        }
        let fvt = *(factory as *const *const IDXGIFactory1Vtbl);
        for index in 0..16u32 {
            let mut adapter: *mut c_void = ptr::null_mut();
            // DXGI_ERROR_NOT_FOUND (a negative HRESULT) ends the enumeration
            if ((*fvt).EnumAdapters1)(factory, index, &mut adapter) < 0 || adapter.is_null() {
                break;
            }
            let avt = *(adapter as *const *const IDXGIAdapter1Vtbl);
            let mut desc: DXGI_ADAPTER_DESC1 = std::mem::zeroed();
            if ((*avt).GetDesc1)(adapter, &mut desc) >= 0 && desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE == 0 {
                out.push(Adapter {
                    luid: format!(
                        "0x{:08x}_0x{:08x}",
                        desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
                    ),
                    name: wide_to_string(&desc.Description),
                    vendor: desc.VendorId,
                    dedicated: desc.DedicatedVideoMemory as u64,
                    shared: desc.SharedSystemMemory as u64,
                });
            }
            ((*avt).Release)(adapter);
        }
        ((*fvt).Release)(factory);
    }
    out
}

unsafe fn add_counter(query: HANDLE, path: &str) -> HANDLE {
    let path = wide(path);
    let mut counter: HANDLE = ptr::null_mut();
    if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) == ERROR_SUCCESS {
        counter
    } else {
        ptr::null_mut()
    }
}

/// Every instance of a wildcard counter as (instance name, value).
fn read_array(counter: HANDLE, large: bool) -> Vec<(String, f64)> {
    if counter.is_null() {
        return Vec::new();
    }
    let format = if large { PDH_FMT_LARGE } else { PDH_FMT_DOUBLE | PDH_FMT_NOCAP100 };
    unsafe {
        // Instances can appear between the sizing call and the real one, so retry
        for _ in 0..3 {
            let mut size: u32 = 0;
            let mut count: u32 = 0;
            let status = PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, ptr::null_mut());
            if status != PDH_MORE_DATA || size == 0 {
                return Vec::new();
            }
            // u64 storage keeps the item array 8-byte aligned
            let mut buf: Vec<u64> = vec![0; (size as usize + 7) / 8];
            let items_ptr = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            let status = PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, items_ptr);
            if status == PDH_MORE_DATA {
                continue;
            }
            if status != ERROR_SUCCESS {
                return Vec::new();
            }
            let items = std::slice::from_raw_parts(items_ptr as *const PDH_FMT_COUNTERVALUE_ITEM_W, count as usize);
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let cstatus = item.FmtValue.CStatus;
                if cstatus != PDH_CSTATUS_VALID_DATA && cstatus != PDH_CSTATUS_NEW_DATA {
                    continue;
                }
                let value = if large {
                    item.FmtValue.bits as i64 as f64
                } else {
                    f64::from_bits(item.FmtValue.bits)
                };
                out.push((wide_ptr_to_string(item.szName), value));
            }
            return out;
        }
    }
    Vec::new()
}

fn per_luid(samples: Vec<(String, f64)>) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for (instance, value) in samples {
        if let Some(luid) = pdh_luid(&instance) {
            *out.entry(luid).or_insert(0) += value.max(0.0) as u64;
        }
    }
    out
}

pub struct PdhGpu {
    query: HANDLE,
    engine: HANDLE,
    dedicated: HANDLE,
    shared: HANDLE,
    adapters: Vec<Adapter>,
    skip_vendor: Option<u32>,
}

impl PdhGpu {
    /// `skip_vendor` drops adapters another source already reports (NVIDIA via NVML).
    pub fn new(skip_vendor: Option<u32>) -> Option<Self> {
        let adapters = dxgi_adapters();
        unsafe {
            let mut query: HANDLE = ptr::null_mut();
            if PdhOpenQueryW(ptr::null(), 0, &mut query) != ERROR_SUCCESS || query.is_null() {
                return None;
            }
            let engine = add_counter(query, r"\GPU Engine(*)\Utilization Percentage");
            let dedicated = add_counter(query, r"\GPU Adapter Memory(*)\Dedicated Usage");
            let shared = add_counter(query, r"\GPU Adapter Memory(*)\Shared Usage");
            if engine.is_null() && dedicated.is_null() {
                PdhCloseQuery(query);
                return None;
            }
            // Utilisation is a rate counter: the first collection only primes it
            PdhCollectQueryData(query);
            Some(Self { query, engine, dedicated, shared, adapters, skip_vendor })
        }
    }

    pub fn sample(&mut self) -> Vec<GpuReading> {
        if unsafe { PdhCollectQueryData(self.query) } != ERROR_SUCCESS {
            return Vec::new();
        }
        let util = adapter_utilisation(&read_array(self.engine, false));
        let dedicated = per_luid(read_array(self.dedicated, true));
        let shared = per_luid(read_array(self.shared, true));

        let mut out = Vec::new();
        if self.adapters.is_empty() {
            // DXGI unavailable: still show whatever the counters know, unnamed
            let mut luids: Vec<&String> = util.keys().collect();
            luids.sort();
            for (i, luid) in luids.into_iter().enumerate() {
                out.push(GpuReading {
                    key: format!("pdh:{}", luid),
                    name: format!("GPU {}", i),
                    backend: "Windows",
                    util: util.get(luid).copied(),
                    vram_used: dedicated.get(luid).copied(),
                    ..Default::default()
                });
            }
            return out;
        }

        for adapter in &self.adapters {
            if Some(adapter.vendor) == self.skip_vendor {
                continue;
            }
            let (used, total) = if adapter.dedicated >= DEDICATED_MIN {
                (dedicated.get(&adapter.luid).copied(), Some(adapter.dedicated))
            } else {
                (shared.get(&adapter.luid).copied(), Some(adapter.shared))
            };
            out.push(GpuReading {
                key: format!("pdh:{}", adapter.luid),
                name: adapter.name.clone(),
                backend: "Windows",
                // An idle adapter has no engine instances at all, which means 0 %
                util: Some(util.get(&adapter.luid).copied().unwrap_or(0.0)),
                vram_used: used,
                vram_total: total,
                temp: None,
                clock_mhz: None,
            });
        }
        out
    }
}

impl Drop for PdhGpu {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
