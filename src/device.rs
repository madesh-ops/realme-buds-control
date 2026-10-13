use crate::proto::{self, *};
use crate::win::{self, CodecInfo, RfcommSocket};
use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const DEVICE_NAME: &str = "T310";
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_secs(3);

pub struct Earbuds {
    pub address: u64,
    sock: RfcommSocket,
    parser: FrameParser,
    seq: u8,
}

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg.into())
}

pub fn find_paired() -> Option<u64> {
    win::paired_devices().into_iter().find(|d| d.name.contains(DEVICE_NAME)).map(|d| d.address)
}

impl Earbuds {
    pub fn connect(address: u64) -> io::Result<Self> {
        let sock = RfcommSocket::connect(address, win::SPP_UUID)?;
        sock.set_read_timeout(Duration::from_millis(200))?;
        Ok(Earbuds { address, sock, parser: FrameParser::default(), seq: 0 })
    }

    /// Sends a command and returns the payload of its response (status byte included).
    pub fn request(&mut self, cmd: u16, data: &[u8]) -> io::Result<Vec<u8>> {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        self.sock.write_all(&build_frame(cmd, seq, data))?;
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        let mut buf = [0u8; 2048];
        while Instant::now() < deadline {
            let n = self.sock.read(&mut buf)?;
            for pkt in self.parser.push(&buf[..n]) {
                if pkt.cmd == cmd | RESPONSE_BIT {
                    return Ok(pkt.data);
                }
            }
        }
        Err(err(format!("no response to command 0x{cmd:04X}")))
    }

    fn request_ok(&mut self, cmd: u16, data: &[u8]) -> io::Result<Vec<u8>> {
        let r = self.request(cmd, data)?;
        match r.first() {
            Some(0) => Ok(r),
            s => Err(err(format!("command 0x{cmd:04X} rejected (status {s:?})"))),
        }
    }

    pub fn get_mode(&mut self) -> io::Result<u8> {
        let r = self.request_ok(CMD_GET_NOISE, &[1, 1])?;
        r.get(3).or(r.last()).copied().ok_or_else(|| err("short noise response"))
    }

    pub fn set_mode(&mut self, mode: u8) -> io::Result<()> {
        self.request_ok(CMD_SET_NOISE, &[1, 1, mode]).map(|_| ())
    }

    pub fn battery(&mut self) -> io::Result<Battery> {
        parse_battery(&self.request(CMD_BATTERY, &[])?).ok_or_else(|| err("bad battery response"))
    }

    pub fn firmware(&mut self) -> io::Result<String> {
        parse_firmware(&self.request(CMD_FIRMWARE, &[])?).ok_or_else(|| err("bad firmware response"))
    }

    pub fn current_eq(&mut self) -> io::Result<u8> {
        self.request_ok(CMD_GET_EQ, &[])?.get(1).copied().ok_or_else(|| err("short EQ response"))
    }

    pub fn custom_eqs(&mut self) -> io::Result<Vec<EqProfile>> {
        parse_eq_list(&self.request(CMD_GET_ALL_EQ, &[])?).ok_or_else(|| err("bad EQ list"))
    }

    pub fn select_eq(&mut self, id: u8) -> io::Result<()> {
        self.request_ok(CMD_SELECT_EQ, &[id]).map(|_| ())
    }

    /// Returns the id the earbuds assigned (add) or echoed (modify/delete).
    pub fn custom_eq(&mut self, action: u8, id: u8, name: &str, bands: &[(u16, i8)]) -> io::Result<u8> {
        let r = self.request_ok(CMD_SET_CUSTOM_EQ, &custom_eq_payload(action, id, name, bands))?;
        Ok(r.get(1).copied().unwrap_or(id))
    }
}

// ---- background worker ------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Request {
    Connect,
    SetMode(u8),
    SelectEq(u8),
    AddCustomEq { name: String, bands: Vec<(u16, i8)> },
    SaveCustomEq { id: u8, name: String, bands: Vec<(u16, i8)> },
    DeleteCustomEq { id: u8, name: String },
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub connected: bool,
    pub busy: bool,
    pub address: Option<u64>,
    pub message: String,
    pub mode: Option<u8>,
    pub battery: Battery,
    pub firmware: Option<String>,
    pub eq: Option<u8>,
    pub custom_eqs: Vec<EqProfile>,
    pub codec: CodecInfo,
}

pub fn run_worker(requests: Receiver<Request>, publish: impl Fn(Snapshot)) {
    let mut state = Snapshot { message: "Not connected".into(), ..Default::default() };
    let mut buds: Option<Earbuds> = None;
    let mut pending = Some(Request::Connect);

    loop {
        let req = match pending.take() {
            Some(r) => Some(r),
            None => match requests.recv_timeout(POLL_INTERVAL) {
                Ok(r) => Some(r),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
        };

        if let Some(addr) = state.address {
            state.codec = win::a2dp_codec(addr);
        }

        // explicit Connect always reconnects; any other request reconnects only when needed
        let reconnect = matches!(req, Some(Request::Connect)) || (buds.is_none() && req.is_some());
        if reconnect {
            buds = None;
            state.busy = true;
            state.message = "Connecting…".into();
            publish(state.clone());
            match find_paired() {
                None => {
                    state.message = "realme Buds T310 is not paired with this PC".into();
                }
                Some(addr) => {
                    state.address = Some(addr);
                    state.codec = win::a2dp_codec(addr);
                    match Earbuds::connect(addr) {
                        Ok(mut b) => {
                            state.firmware = b.firmware().ok();
                            buds = Some(b);
                            state.connected = true;
                            state.message = format!("Connected to {}", win::format_addr(addr));
                        }
                        Err(e) => state.message = format!("Could not connect: {e}"),
                    }
                }
            }
        }

        if let Some(b) = buds.as_mut() {
            let result = (|| -> io::Result<()> {
                match &req {
                    Some(Request::SetMode(m)) => b.set_mode(*m)?,
                    Some(Request::SelectEq(id)) => b.select_eq(*id)?,
                    Some(Request::AddCustomEq { name, bands }) => {
                        b.custom_eq(EQ_ADD, 0, name, bands)?;
                    }
                    Some(Request::SaveCustomEq { id, name, bands }) => {
                        b.custom_eq(EQ_MODIFY, *id, name, bands)?;
                        b.select_eq(*id)?;
                    }
                    Some(Request::DeleteCustomEq { id, name }) => {
                        let bands: Vec<(u16, i8)> = DEFAULT_EQ_FREQS.iter().map(|&f| (f, 0)).collect();
                        b.custom_eq(EQ_DELETE, *id, name, &bands)?;
                        // the earbuds fall back to an arbitrary preset; use Nature Balance instead
                        b.select_eq(0)?;
                    }
                    _ => {}
                }
                state.mode = Some(b.get_mode()?);
                state.battery = b.battery()?;
                state.eq = Some(b.current_eq()?);
                state.custom_eqs = b.custom_eqs()?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    state.connected = true;
                    if req.is_some() {
                        state.message = format!("Connected to {}", win::format_addr(b.address));
                    }
                }
                Err(e) => {
                    buds = None;
                    state.connected = false;
                    state.mode = None;
                    state.eq = None;
                    state.message = format!("Disconnected: {e}");
                }
            }
        }
        state.busy = false;
        publish(state.clone());
    }
}

pub fn describe(s: &Snapshot) -> String {
    let lvl = |l: Option<BatteryLevel>| match l {
        Some(l) => format!("{}%{}", l.percent, if l.charging { " (charging)" } else { "" }),
        None => "-".into(),
    };
    let codec = match s.codec.active {
        Some(c) => format!(
            "{} (earbuds support: {})",
            win::codec_name(c),
            s.codec.available.iter().map(|&c| win::codec_name(c)).collect::<Vec<_>>().join(", ")
        ),
        None => "not streaming to this PC".into(),
    };
    format!(
        "Status:   {}\nMode:     {}\nEQ:       {}\nBattery:  left {}, right {}, case {}\nCodec:    {}\nFirmware: {}",
        s.message,
        s.mode.map(proto::mode_name).unwrap_or("-"),
        s.eq.map(|id| proto::eq_name(id, &s.custom_eqs)).unwrap_or_else(|| "-".into()),
        lvl(s.battery.left),
        lvl(s.battery.right),
        lvl(s.battery.case),
        codec,
        s.firmware.as_deref().unwrap_or("-"),
    )
}
