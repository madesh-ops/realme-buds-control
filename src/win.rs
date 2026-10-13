//! Thin Win32 bindings: Winsock RFCOMM sockets, paired-device lookup and the
//! A2DP codec property published by BthA2dp.sys.
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::io;
use std::time::Duration;

type Socket = usize;
const INVALID_SOCKET: Socket = usize::MAX;
const AF_BTH: i32 = 32;
const SOCK_STREAM: i32 = 1;
const BTHPROTO_RFCOMM: i32 = 3;
const SOL_SOCKET: i32 = 0xFFFF;
const SO_RCVTIMEO: i32 = 0x1006;
const WSAETIMEDOUT: i32 = 10060;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl Guid {
    pub const fn new(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Self {
        Guid { data1: d1, data2: d2, data3: d3, data4: d4 }
    }
    fn to_bytes(self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&self.data1.to_le_bytes());
        b[4..6].copy_from_slice(&self.data2.to_le_bytes());
        b[6..8].copy_from_slice(&self.data3.to_le_bytes());
        b[8..].copy_from_slice(&self.data4);
        b
    }
}

/// Standard Serial Port Profile; the T310 config uses TYPE_DEFAULT -> SPP.
pub const SPP_UUID: Guid = Guid::new(0x0000_1101, 0x0000, 0x1000, [0x80, 0x00, 0x00, 0x80, 0x5F, 0x9B, 0x34, 0xFB]);

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSAGetLastError() -> i32;
    fn socket(af: i32, ty: i32, protocol: i32) -> Socket;
    fn connect(s: Socket, name: *const u8, namelen: i32) -> i32;
    fn send(s: Socket, buf: *const u8, len: i32, flags: i32) -> i32;
    fn recv(s: Socket, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn setsockopt(s: Socket, level: i32, name: i32, val: *const u8, len: i32) -> i32;
    fn closesocket(s: Socket) -> i32;
}

fn wsa_error(what: &str) -> io::Error {
    let code = unsafe { WSAGetLastError() };
    io::Error::new(io::ErrorKind::Other, format!("{what} failed (WSA error {code})"))
}

pub struct RfcommSocket(Socket);

impl RfcommSocket {
    /// Connects to `service` on the device; Windows resolves the RFCOMM channel through SDP.
    pub fn connect(addr: u64, service: Guid) -> io::Result<Self> {
        unsafe {
            let mut wsa = [0u8; 512];
            WSAStartup(0x0202, wsa.as_mut_ptr());
            let s = socket(AF_BTH, SOCK_STREAM, BTHPROTO_RFCOMM);
            if s == INVALID_SOCKET {
                return Err(wsa_error("socket()"));
            }
            // SOCKADDR_BTH is packed: family u16, btAddr u64, serviceClassId GUID, port u32
            let mut sa = [0u8; 30];
            sa[0..2].copy_from_slice(&(AF_BTH as u16).to_le_bytes());
            sa[2..10].copy_from_slice(&addr.to_le_bytes());
            sa[10..26].copy_from_slice(&service.to_bytes());
            if connect(s, sa.as_ptr(), sa.len() as i32) != 0 {
                let e = wsa_error("RFCOMM connect");
                closesocket(s);
                return Err(e);
            }
            Ok(RfcommSocket(s))
        }
    }

    pub fn set_read_timeout(&self, t: Duration) -> io::Result<()> {
        let ms = (t.as_millis().max(1) as u32).to_le_bytes();
        if unsafe { setsockopt(self.0, SOL_SOCKET, SO_RCVTIMEO, ms.as_ptr(), 4) } != 0 {
            return Err(wsa_error("setsockopt"));
        }
        Ok(())
    }

    pub fn write_all(&self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let n = unsafe { send(self.0, buf.as_ptr(), buf.len() as i32, 0) };
            if n <= 0 {
                return Err(wsa_error("send"));
            }
            buf = &buf[n as usize..];
        }
        Ok(())
    }

    /// Returns Ok(0) on timeout.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe { recv(self.0, buf.as_mut_ptr(), buf.len() as i32, 0) };
        if n > 0 {
            return Ok(n as usize);
        }
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "earbuds closed the connection"));
        }
        if unsafe { WSAGetLastError() } == WSAETIMEDOUT {
            return Ok(0);
        }
        Err(wsa_error("recv"))
    }
}

impl Drop for RfcommSocket {
    fn drop(&mut self) {
        unsafe { closesocket(self.0) };
    }
}

// ---- paired devices (BluetoothApis) -------------------------------------

#[repr(C)]
struct SearchParams {
    dw_size: u32,
    return_authenticated: i32,
    return_remembered: i32,
    return_unknown: i32,
    return_connected: i32,
    issue_inquiry: i32,
    timeout_multiplier: u8,
    radio: *mut c_void,
}

#[repr(C)]
struct DeviceInfo {
    dw_size: u32,
    address: u64,
    class_of_device: u32,
    connected: i32,
    remembered: i32,
    authenticated: i32,
    last_seen: [u16; 8],
    last_used: [u16; 8],
    name: [u16; 248],
}

#[link(name = "BluetoothApis", kind = "raw-dylib")]
unsafe extern "system" {
    fn BluetoothFindFirstDevice(params: *const SearchParams, info: *mut DeviceInfo) -> *mut c_void;
    fn BluetoothFindNextDevice(find: *mut c_void, info: *mut DeviceInfo) -> i32;
    fn BluetoothFindDeviceClose(find: *mut c_void) -> i32;
}

pub struct PairedDevice {
    pub name: String,
    pub address: u64,
}

pub fn paired_devices() -> Vec<PairedDevice> {
    let mut out = Vec::new();
    unsafe {
        let params = SearchParams {
            dw_size: size_of::<SearchParams>() as u32,
            return_authenticated: 1,
            return_remembered: 1,
            return_unknown: 0,
            return_connected: 1,
            issue_inquiry: 0,
            timeout_multiplier: 0,
            radio: std::ptr::null_mut(),
        };
        let mut info: DeviceInfo = std::mem::zeroed();
        info.dw_size = size_of::<DeviceInfo>() as u32;
        let h = BluetoothFindFirstDevice(&params, &mut info);
        if h.is_null() {
            return out;
        }
        loop {
            let len = info.name.iter().position(|&c| c == 0).unwrap_or(info.name.len());
            out.push(PairedDevice { name: String::from_utf16_lossy(&info.name[..len]), address: info.address });
            if BluetoothFindNextDevice(h, &mut info) == 0 {
                break;
            }
        }
        BluetoothFindDeviceClose(h);
    }
    out
}

pub fn format_addr(a: u64) -> String {
    (0..6).rev().map(|i| format!("{:02X}", (a >> (8 * i)) & 0xFF)).collect::<Vec<_>>().join(":")
}


// ---- A2DP codec (cfgmgr32) ----------------------------------------------

#[repr(C)]
struct DevPropKey {
    fmtid: Guid,
    pid: u32,
}

#[link(name = "cfgmgr32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CM_Get_Device_Interface_List_SizeW(len: *mut u32, class: *const Guid, device_id: *const u16, flags: u32) -> u32;
    fn CM_Get_Device_Interface_ListW(class: *const Guid, device_id: *const u16, buf: *mut u16, len: u32, flags: u32) -> u32;
    fn CM_Get_Device_Interface_PropertyW(
        iface: *const u16,
        key: *const DevPropKey,
        ty: *mut u32,
        buf: *mut u8,
        size: *mut u32,
        flags: u32,
    ) -> u32;
}

const KSCATEGORY_AUDIO: Guid = Guid::new(0x6994_AD04, 0x93EF, 0x11D0, [0xA3, 0xCC, 0x00, 0xA0, 0xC9, 0x22, 0x31, 0x96]);
/// Property set written by BthA2dp.sys on the A2DP sink's KS audio interface.
/// pid 3 = configured codec id, pid 4 = codec ids the earbuds advertise.
const A2DP_CODEC_PROPS: Guid = Guid::new(0x29CE_83D4, 0x7A82, 0x4744, [0xBD, 0x1D, 0xAB, 0xEC, 0x85, 0x32, 0x1D, 0xD6]);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodecInfo {
    pub active: Option<u8>,
    pub available: Vec<u8>,
}

pub fn codec_name(id: u8) -> &'static str {
    // A2DP media codec types (Bluetooth Assigned Numbers)
    match id {
        0x00 => "SBC",
        0x01 => "MP3",
        0x02 => "AAC",
        0x04 => "ATRAC",
        0xFF => "Vendor (aptX/LDAC)",
        _ => "Unknown",
    }
}

fn interface_list(class: &Guid) -> Vec<String> {
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_Interface_List_SizeW(&mut len, class, std::ptr::null(), 0) != 0 || len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(class, std::ptr::null(), buf.as_mut_ptr(), len, 0) != 0 {
            return Vec::new();
        }
        buf.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
    }
}

fn interface_property(iface: &str, pid: u32) -> Option<Vec<u8>> {
    let wide: Vec<u16> = iface.encode_utf16().chain(Some(0)).collect();
    let key = DevPropKey { fmtid: A2DP_CODEC_PROPS, pid };
    unsafe {
        let (mut ty, mut size) = (0u32, 0u32);
        CM_Get_Device_Interface_PropertyW(wide.as_ptr(), &key, &mut ty, std::ptr::null_mut(), &mut size, 0);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if CM_Get_Device_Interface_PropertyW(wide.as_ptr(), &key, &mut ty, buf.as_mut_ptr(), &mut size, 0) != 0 {
            return None;
        }
        buf.truncate(size as usize);
        Some(buf)
    }
}

/// Codec Windows negotiated with the earbuds for the PC's A2DP stream.
pub fn a2dp_codec(addr: u64) -> CodecInfo {
    let needle = format!("{addr:012x}");
    let Some(iface) = interface_list(&KSCATEGORY_AUDIO)
        .into_iter()
        .find(|p| p.to_ascii_lowercase().contains(&needle) && p.to_ascii_lowercase().contains("0000110b"))
    else {
        return CodecInfo::default();
    };
    CodecInfo {
        active: interface_property(&iface, 3).and_then(|b| b.first().copied()),
        available: interface_property(&iface, 4).unwrap_or_default(),
    }
}

// ---- accessibility ----------------------------------------------------------

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, win_ini: u32) -> i32;
}

/// Settings > Accessibility > Visual effects > Animation effects.
pub fn animations_enabled() -> bool {
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut on: i32 = 1;
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut i32).cast(), 0) };
    ok == 0 || on != 0
}

// ---- console for CLI use from a GUI-subsystem exe ---------------------------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AttachConsole(pid: u32) -> i32;
}

pub fn attach_parent_console() {
    unsafe { AttachConsole(u32::MAX) };
}
