//! realme "TL" earbud protocol, reverse-engineered from realme Link 5.8.811
//! (com.realme.iot.headset.tl). Packets are wrapped in OPPOv1 frames and sent
//! over an RFCOMM (SPP) socket.
//!
//! Frame:  AA <varint len> <ctrl> 00 <packet>          (len = packet.len() + 2)
//! Packet: <cmd u16 LE> <seq u8> <payload len u16 LE> <payload>
//! A response carries the request's command id with bit 15 set; its first
//! payload byte is a status code (0 = success).

pub const CMD_FIRMWARE: u16 = 0x0105;
pub const CMD_BATTERY: u16 = 0x0106;
pub const CMD_GET_NOISE: u16 = 0x010C;
pub const CMD_GET_EQ: u16 = 0x010F;
pub const CMD_GET_ALL_EQ: u16 = 0x0122;
pub const CMD_SET_NOISE: u16 = 0x0404;
pub const CMD_SELECT_EQ: u16 = 0x0406;
pub const CMD_SET_CUSTOM_EQ: u16 = 0x0418;

pub const RESPONSE_BIT: u16 = 0x8000;

/// Noise-control values as sent on the wire.
pub const MODE_NORMAL: u8 = 0x01;
pub const MODE_TRANSPARENCY: u8 = 0x02;
pub const MODE_ANC: u8 = 0x08;

/// Custom-EQ actions (SetEqInfoAction in the app).
pub const EQ_ADD: u8 = 1;
pub const EQ_MODIFY: u8 = 2;
pub const EQ_DELETE: u8 = 3;

/// Built-in EQ presets; ids >= 4 are custom profiles.
pub const PRESETS: [(u8, &str); 4] = [
    (0, "Nature Balance"),
    (1, "Bass Boost"),
    (2, "Clear Vocals"),
    (3, "Clear Bass"),
];
pub const MAX_CUSTOM_EQS: usize = 3;
pub const EQ_MIN_DB: i8 = -6;
pub const EQ_MAX_DB: i8 = 6;
/// Bands used when the earbuds report none; the T310 stores 50 Hz for the first band.
pub const DEFAULT_EQ_FREQS: [u16; 6] = [50, 250, 1000, 4000, 8000, 16000];

pub fn mode_name(mode: u8) -> &'static str {
    match mode {
        MODE_NORMAL => "Normal",
        MODE_TRANSPARENCY => "Transparency",
        MODE_ANC => "Noise Cancellation",
        0x04 => "ANC (mild)",
        0x10 => "ANC (moderate)",
        0x20 => "ANC (smart)",
        _ => "Unknown",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub cmd: u16,
    pub seq: u8,
    pub data: Vec<u8>,
}

fn varint(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

pub fn build_frame(cmd: u16, seq: u8, data: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(5 + data.len());
    packet.extend_from_slice(&cmd.to_le_bytes());
    packet.push(seq);
    packet.extend_from_slice(&(data.len() as u16).to_le_bytes());
    packet.extend_from_slice(data);

    let mut frame = vec![0xAA];
    varint(packet.len() + 2, &mut frame);
    frame.extend_from_slice(&[0x00, 0x00]);
    frame.extend_from_slice(&packet);
    frame
}

/// Reassembles frames from a byte stream.
#[derive(Default)]
pub struct FrameParser {
    buf: Vec<u8>,
}

impl FrameParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Packet> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(start) = self.buf.iter().position(|&b| b == 0xAA) else {
                self.buf.clear();
                return out;
            };
            self.buf.drain(..start);

            // varint length after the start byte
            let (mut len, mut shift, mut i) = (0usize, 0, 1);
            loop {
                let Some(&b) = self.buf.get(i) else { return out };
                len |= ((b & 0x7F) as usize) << shift;
                i += 1;
                if b & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            if self.buf.len() < i + len {
                return out;
            }
            let body: Vec<u8> = self.buf[i..i + len].to_vec();
            self.buf.drain(..i + len);
            if body.len() < 2 {
                continue;
            }
            // multi-frame messages carry an extra sequence byte; the T310 only sends single frames
            let pkt = if body[0] & 0x03 != 0 { &body[3.min(body.len())..] } else { &body[2..] };
            if pkt.len() >= 5 {
                let plen = u16::from_le_bytes([pkt[3], pkt[4]]) as usize;
                let end = (5 + plen).min(pkt.len());
                out.push(Packet {
                    cmd: u16::from_le_bytes([pkt[0], pkt[1]]),
                    seq: pkt[2],
                    data: pkt[5..end].to_vec(),
                });
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BatteryLevel {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Battery {
    pub left: Option<BatteryLevel>,
    pub right: Option<BatteryLevel>,
    pub case: Option<BatteryLevel>,
}

/// Response to CMD_BATTERY: [status][count]([type][level|charging<<7])*. Types: 1 left, 2 right, 3 case.
pub fn parse_battery(data: &[u8]) -> Option<Battery> {
    if data.len() < 2 || data[0] != 0 {
        return None;
    }
    let mut bat = Battery::default();
    for pair in data[2..].chunks_exact(2).take(data[1] as usize) {
        let level = BatteryLevel { percent: pair[1] & 0x7F, charging: pair[1] & 0x80 != 0 };
        match pair[0] {
            1 => bat.left = Some(level),
            2 => bat.right = Some(level),
            3 => bat.case = Some(level),
            _ => {}
        }
    }
    Some(bat)
}

/// Response to CMD_FIRMWARE: [status][count] then ASCII "type,len,version,type,len,version".
pub fn parse_firmware(data: &[u8]) -> Option<String> {
    if data.len() < 3 || data[0] != 0 {
        return None;
    }
    let text = String::from_utf8_lossy(&data[2..]);
    let mut versions: Vec<&str> = text.split(',').filter(|s| s.contains('.')).collect();
    versions.dedup();
    Some(versions.join(" / "))
}

#[derive(Debug, Clone, PartialEq)]
pub struct EqProfile {
    pub id: u8,
    pub name: String,
    pub selected: bool,
    pub min_db: i8,
    pub max_db: i8,
    pub bands: Vec<(u16, i8)>,
}

/// Response to CMD_GET_ALL_EQ: [status][count] then per profile
/// [selected][min][max][id][name len][name][band count]([freq u16 LE][gain i8])*.
pub fn parse_eq_list(data: &[u8]) -> Option<Vec<EqProfile>> {
    if data.len() < 2 || data[0] != 0 {
        return None;
    }
    let count = data[1] as usize;
    let mut p = 2;
    let mut out = Vec::with_capacity(count);
    let get = |i: usize| data.get(i).copied();
    for _ in 0..count {
        let selected = get(p)? != 0;
        let min_db = get(p + 1)? as i8;
        let max_db = get(p + 2)? as i8;
        let id = get(p + 3)?;
        let name_len = get(p + 4)? as usize;
        p += 5;
        let name = String::from_utf8_lossy(data.get(p..p + name_len)?).into_owned();
        p += name_len;
        let n = get(p)? as usize;
        p += 1;
        let mut bands = Vec::with_capacity(n);
        for _ in 0..n {
            let b = data.get(p..p + 3)?;
            bands.push((u16::from_le_bytes([b[0], b[1]]), b[2] as i8));
            p += 3;
        }
        out.push(EqProfile { id, name, selected, min_db, max_db, bands });
    }
    Some(out)
}

pub fn custom_eq_payload(action: u8, id: u8, name: &str, bands: &[(u16, i8)]) -> Vec<u8> {
    let name = name.as_bytes();
    let mut b = vec![action, EQ_MIN_DB as u8, EQ_MAX_DB as u8, id, name.len() as u8];
    b.extend_from_slice(name);
    b.push(bands.len() as u8);
    for &(freq, gain) in bands {
        b.extend_from_slice(&freq.to_le_bytes());
        b.push(gain.clamp(EQ_MIN_DB, EQ_MAX_DB) as u8);
    }
    b
}

pub fn eq_name(id: u8, customs: &[EqProfile]) -> String {
    if let Some((_, n)) = PRESETS.iter().find(|(p, _)| *p == id) {
        return n.to_string();
    }
    customs.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_else(|| format!("EQ {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_matches_app() {
        // AA 0A 00 00 | 04 04 05 03 00 | 01 01 08  (set ANC, seq 5)
        assert_eq!(
            build_frame(CMD_SET_NOISE, 5, &[1, 1, MODE_ANC]),
            [0xAA, 0x0A, 0, 0, 0x04, 0x04, 5, 3, 0, 1, 1, 8]
        );
    }

    #[test]
    fn parser_roundtrip_and_split() {
        let mut p = FrameParser::default();
        let f = build_frame(0x8106, 9, &[0, 3, 2, 0x64, 1, 0xE2, 3, 0]);
        assert!(p.push(&f[..4]).is_empty());
        let pk = p.push(&f[4..]);
        assert_eq!(pk, vec![Packet { cmd: 0x8106, seq: 9, data: vec![0, 3, 2, 0x64, 1, 0xE2, 3, 0] }]);
        let b = parse_battery(&pk[0].data).unwrap();
        assert_eq!(b.right, Some(BatteryLevel { percent: 100, charging: false }));
        assert_eq!(b.left, Some(BatteryLevel { percent: 98, charging: true }));
        assert_eq!(b.case, Some(BatteryLevel { percent: 0, charging: false }));
    }

    #[test]
    fn eq_list_from_device_capture() {
        // captured from a T310 after adding "ClaudeTest"
        let hex = "00 01 01 fa 06 04 0a 43 6c 61 75 64 65 54 65 73 74 06 32 00 02 fa 00 01 e8 03 00 a0 0f 00 40 1f 01 80 3e 02";
        let data: Vec<u8> = hex.split(' ').map(|h| u8::from_str_radix(h, 16).unwrap()).collect();
        let l = parse_eq_list(&data).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, 4);
        assert_eq!(l[0].name, "ClaudeTest");
        assert_eq!((l[0].min_db, l[0].max_db), (-6, 6));
        assert_eq!(l[0].bands[0], (50, 2));
        assert_eq!(l[0].bands[5], (16000, 2));
        let payload = custom_eq_payload(EQ_MODIFY, 4, "ClaudeTest", &l[0].bands);
        assert_eq!(&payload[..5], &[2, 0xFA, 6, 4, 10]);
    }

    #[test]
    fn firmware_versions() {
        let mut d = vec![0, 2];
        d.extend_from_slice(b"1,2,1.1.0.45,2,2,1.1.0.45");
        assert_eq!(parse_firmware(&d).unwrap(), "1.1.0.45");
    }
}
