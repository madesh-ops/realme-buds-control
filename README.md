# T310 Control

A small native Windows app for the **realme Buds T310**: switch noise control, pick or build EQ profiles, and see battery levels and the Bluetooth codec Windows is using. No phone or realme account needed.

> Not affiliated with or endorsed by realme. "realme" is a trademark of its owner.

## Features

- **Noise control:** Noise Cancellation, Transparency, Off
- **EQ presets:** Nature Balance, Bass Boost, Clear Vocals, Clear Bass
- **Custom EQ:** up to three named profiles stored on the earbuds, six bands (50 Hz – 16 kHz, ±6 dB)
- **Battery:** left bud, right bud and case, with charging state
- **Codec:** the A2DP codec Windows negotiated with the earbuds (SBC or AAC) and the codecs they support
- **Firmware version** and Bluetooth address
- Follows the Windows light/dark theme and respects *Animation effects* being turned off
- A command-line mode for scripts and shortcuts

## Install

1. Pair the T310 with Windows (*Settings → Bluetooth & devices → Add device*).
2. Download `T310-Control-Setup-x.y.z.exe` from [Releases](../../releases) and run it. It installs for the current user only and doesn't need administrator rights.
3. Start **T310 Control** from the Start menu.

If the app can't connect, close **realme Link** on your phone. The earbuds accept only one control connection at a time.

To uninstall, use *Settings → Apps → Installed apps → T310 Control*.

## Command line

```
t310_control status
t310_control mode anc | transparency | normal
t310_control eq <0-3 | custom id>
t310_control custom add <name> [g1 … g6]
t310_control custom set <id> <name> [g1 … g6]
t310_control custom del <id>
```

Gains are whole dB from −6 to +6 for the bands 50, 250, 1k, 4k, 8k and 16k Hz.

## Compatibility

Tested on the **realme Buds T310** (firmware 1.1.0.45) with Windows 11.

Other realme earbuds that realme Link drives with the same "TL" protocol (for example Buds Air 5/6/7, Buds T300, T200 and N1) use the same commands. The app doesn't support them yet:

- it looks for a paired device whose name contains `T310`
- some models use a different RFCOMM service UUID
- EQ preset names, ANC strength levels and custom-EQ bands differ between models

Older Airoha-based models (such as the original Buds Air) use a different protocol entirely.

## How it works

The app opens an RFCOMM socket to the earbuds' Serial Port Profile service (`00001101-0000-1000-8000-00805F9B34FB`) and speaks the same protocol as the realme Link Android app:

```
frame:   AA <varint len> 00 00 <packet>             len = packet length + 2
packet:  <command u16 LE> <seq u8> <length u16 LE> <payload>
```

A response carries the command id with bit 15 set, and its first payload byte is a status code (0 = success).

| Command | Purpose | Payload |
|---|---|---|
| `0x0404` | Set noise control | `01 01 <mode>`, mode `08` ANC, `02` Transparency, `01` Off |
| `0x010C` | Get noise control | `01 01` |
| `0x0406` | Select EQ | `<id>` (0–3 presets, 4+ custom) |
| `0x010F` | Get current EQ | – |
| `0x0122` | List custom EQs | – |
| `0x0418` | Add / modify / delete custom EQ | `<action> <min> <max> <id> <name len> <name> <bands> (<freq u16 LE> <gain>)…` |
| `0x0106` | Battery | – → `<count> (<type> <level \| charging<<7>)…`, type 1 left, 2 right, 3 case |
| `0x0105` | Firmware version | – |

The codec isn't part of that protocol. Windows' Bluetooth A2DP driver (`BthA2dp.sys`) publishes it as a property on the earbuds' audio interface, which the app reads with `CM_Get_Device_Interface_PropertyW`.

## Build from source

Requires Rust (MSVC toolchain) and the Windows SDK.

```
cargo build --release
```

To build the installer, install [NSIS](https://nsis.sourceforge.io/) and run:

```
cargo build --release --target-dir dist-build
makensis /DVERSION=0.1.0 installer\setup.nsi
```

The installer is written to `dist\`. `assets/make_icon.py` regenerates the app icon.

## License

[MIT](LICENSE)
