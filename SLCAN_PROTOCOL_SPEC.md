# SLCAN-Compatible Adapter Protocol Specification

This document specifies an SLCAN-compatible classic-CAN adapter profile. Use
it as the complete wire contract when writing a host application or a
compatible adapter implementation.

SLCAN is an ASCII, line-oriented protocol originally used by LAWICEL CAN232
and CANUSB devices. It carries classic CAN frames over a serial-like stream,
such as a USB CDC virtual COM port. This document defines the supported
profile, not every command that has appeared in historical SLCAN dialects.

## Transport And Framing

- The transport is a byte stream. USB CDC, UART, and a virtual serial port are
  suitable transports.
- A host command is ASCII text terminated by carriage return (`CR`, `0x0D`).
  The adapter also accepts line feed (`LF`, `0x0A`) as a command terminator.
- Adapter output always uses `CR` as the terminator. It does not append `LF`.
- Treat every `CR`-terminated sequence as one line. Reads may split a line or
  contain multiple lines; do not assume one transport read is one message.
- The adapter accepts commands up to 31 ASCII characters, excluding the
  terminator. It discards an overlong command through its next terminator and
  then emits a NACK.
- Command letters are case-sensitive. Use the exact case shown below. Hex
  digits in CAN identifiers and data may be upper- or lowercase.

The channel starts closed. The default selected bus is `CAN1`, the default
bitrate code is `S6`, and receive timestamps start enabled.

## Responses And Asynchronous Frames

The device can send received CAN-frame lines at any time while the channel is
open, including while a command response is pending. A host must classify each
complete line instead of assuming that the next line is the command response.

| Result | Wire bytes | Meaning |
| --- | --- | --- |
| ACK | `\r` | Successful non-frame command. After removing the terminator, this is an empty line. |
| TX accepted | `z\r` | A transmit frame was accepted into the adapter transmit queue. It does not confirm arbitration or delivery on the CAN bus. |
| NACK | `\a` | The command was invalid, rejected in its current state, or unsupported. `\a` is ASCII BEL (`0x07`), with no trailing `CR`. |
| Information | `V...\r`, `N...\r`, `XSTAT ...\r`, or `XBRIDGE ...\r` | Command-specific response. |
| Received frame | `t...\r`, `T...\r`, `r...\r`, or `R...\r` | An asynchronous CAN frame described below. |

Because a NACK is a single BEL byte rather than a line, recognize it directly
in the byte stream. Preserve empty `CR`-terminated lines so ACKs are not lost.
Serialize host writes and correlate replies with the command that produced
them. For commands with an ACK, an asynchronous frame may precede the ACK.

## CAN Frame Encoding

SLCAN represents bytes as two hexadecimal characters. `ID` and data fields
are emitted in uppercase by this firmware, but parsers should accept either
hexadecimal case.

| Prefix | CAN format | Identifier field | Body after ID |
| --- | --- | --- | --- |
| `t` | Standard (11-bit) data frame | 3 hex digits, `000`-`7FF` | DLC, then `2 * DLC` data hex digits |
| `T` | Extended (29-bit) data frame | 8 hex digits, `00000000`-`1FFFFFFF` | DLC, then `2 * DLC` data hex digits |
| `r` | Standard (11-bit) RTR frame | 3 hex digits | DLC only |
| `R` | Extended (29-bit) RTR frame | 8 hex digits | DLC only |

`DLC` is one hexadecimal digit in the range `0` through `8`. This profile
supports classic CAN only: up to eight data bytes. It does not define CAN FD,
BRS, ESI, error frames, or a DLC greater than 8.

### Host-To-Device Frames

Transmit commands have no timestamp suffix and require an open channel:

```text
t1232A1B2\r                  ; standard ID 0x123, two bytes: A1 B2
T1ABCDEFF3DEADBE\r            ; extended ID 0x1ABCDEFF, three bytes
r3218\r                       ; standard RTR, ID 0x321, DLC 8
R1ABCDEFF0\r                  ; extended RTR, ID 0x1ABCDEFF, DLC 0
```

Data frames must contain exactly the number of data characters implied by the
DLC. RTR frames must contain no data characters. A malformed frame, a frame
sent while closed, a bus-off condition, or a full host transmit queue returns
NACK. A `z` response means the frame entered the adapter's software transmit
path, not that it was transmitted successfully on CAN.

### Device-To-Host Frames And Timestamps

Received CAN frames use the same format. When timestamps are enabled, append
exactly four hexadecimal digits before the `CR`:

```text
t1232A1B2ABCD\r               ; data frame, timestamp 0xABCD
r3218ABCD\r                   ; RTR frame, timestamp 0xABCD
T1ABCDEFF3DEADBEABCD\r         ; extended data frame, timestamp 0xABCD
```

The timestamp is the low 16 bits of the adapter's millisecond system tick at
CAN-frame enqueue time. It wraps every 65,536 ms. Convert successive values
to a delta with modulo-16-bit arithmetic; do not interpret it as wall-clock
time. With timestamps disabled, the four-digit suffix is absent.

## Standard Command Profile

Commands in this section use the conventional SLCAN command letters and are
supported by this profile.

| Command | Valid state | Success response | Behavior |
| --- | --- | --- | --- |
| `O\r` | Closed | `\r` | Open the selected CAN controller for SLCAN TX and RX. |
| `C\r` | Open | `\r` | Close the selected controller, clear queued SLCAN RX frames, and discard queued host TX frames for that bus. |
| `S0\r` through `S8\r` | Closed | `\r` | Set the selected controller's nominal bitrate. |
| `t...\r`, `T...\r`, `r...\r`, `R...\r` | Open | `z\r` | Queue a CAN frame for transmission. |
| `Z0\r` | Open or closed | `\r` | Disable the timestamp suffix on device-to-host frames. |
| `Z1\r` | Open or closed | `\r` | Enable the timestamp suffix on device-to-host frames. |
| `V\r` | Open or closed | `V1013\r` | Return the profile's fixed version string. Treat the payload as opaque. |
| `N\r` | Open or closed | `N2202\r` | Return the profile's fixed serial-number string. Treat the payload as opaque. |

`O`, `C`, and `Sx` return NACK when invoked in the wrong state or when the
underlying CAN driver rejects the operation. Any unlisted command, including
common dialect commands such as `L`, `A`, `M`, `F`, `P`, and `Q`, is not
implemented by this profile and returns NACK.

### Bitrate Codes

| Code | Nominal bitrate |
| --- | --- |
| `S0` | 10 kbit/s |
| `S1` | 20 kbit/s |
| `S2` | 50 kbit/s |
| `S3` | 100 kbit/s |
| `S4` | 125 kbit/s |
| `S5` | 250 kbit/s |
| `S6` | 500 kbit/s |
| `S7` | 800 kbit/s |
| `S8` | 1 Mbit/s |

Set the bitrate only while closed. A portable open sequence is `Sx\r`, wait
for ACK, then `O\r`, wait for ACK. Close the channel before changing the rate
or selecting another bus.

## Adapter Extensions

The following commands extend conventional SLCAN. Other SLCAN adapters may
return NACK, so applications should feature-detect them and retain a working
standard-SLCAN path.

### Bus Selection

| Command | Valid state | Success response | Behavior |
| --- | --- | --- | --- |
| `Y1\r` | Closed | `\r` | Select `CAN1` as the active SLCAN controller. |
| `Y2\r` | Closed | `\r` | Select `CAN2` as the active SLCAN controller. |

Only one controller is exposed through SLCAN at a time. TX commands use the
selected controller, and only frames received on that controller are reported.
Select a bus with `C\r` (when needed), `Y1\r` or `Y2\r`, optional `Sx\r`, and
then `O\r`.

### Bridge Control

| Command | Success response | Behavior |
| --- | --- | --- |
| `XBRIDGE0\r` | `\r` | Disable CAN1-to-CAN2 bridge forwarding. |
| `XBRIDGE1\r` | `\r` | Enable CAN1-to-CAN2 bridge forwarding. |
| `XBRIDGE?\r` | `XBRIDGE enabled=0\r` or `XBRIDGE enabled=1\r` | Query bridge forwarding state. |

Bridge commands are allowed whether the SLCAN channel is open or closed and
do not change its selected bus, bitrate, or timestamp setting.
Bridge forwarding is enabled after adapter initialization.

### Diagnostics

Send `XSTAT\r` in either channel state to request one CR-terminated status
line. It has this shape, with unsigned decimal values:

```text
XSTAT tx_accepted=<n> tx_rejected=<n> fifo_full=<n> fifo_full_streak_max=<n> cmd_overflow=<n> usb_busy=<n> usb_tx_dropped=<n> bus_off=<n> error_passive=<n> error_warning=<n> protocol_error_arbitration=<n> protocol_error_data=<n> tx_fifo_empty=<n> tx_complete=<n> tx_abort=<n> tx_event_lost=<n> cmd_queue_peak=<n> rx_queue_peak=<n> host_tx_queue_peak=<n> cmd_queue_rejects=<n> rx_queue_rejects=<n> host_tx_queue_rejects=<n> host_tx_age_max_ms=<n>\r
```

Parse this as metadata, never as a CAN frame. The counters are cumulative
since adapter initialization. `tx_accepted` counts frames accepted by the
software TX queue; use `tx_complete`, CAN-driver diagnostics, or application
protocol acknowledgements when delivery confirmation is required.

## Recommended Host State Machine

1. Open the serial transport and start a persistent byte-stream reader.
2. Optionally send `V\r` to confirm that the device responds, then send `Z1\r`
   if device timestamps are wanted. If `Z1` is NACKed, continue using a host
   timestamp rather than failing the connection.
3. While closed, select the optional bus with `Y1\r` or `Y2\r`, then configure
   the bitrate with `Sx\r`.
4. Send `O\r` and wait for its ACK before allowing transmit commands.
5. Continuously route `t`, `T`, `r`, and `R` lines to the RX-frame handler;
   route `XSTAT ` and `XBRIDGE ` lines to extension handlers; and route ACK,
   `z`, and BEL to the pending-command handler.
6. Send `C\r` and wait for ACK before closing the transport or changing bus or
   bitrate.

For a compatible adapter implementation, parse only complete delimiter-terminated
commands, validate identifiers, DLC, and exact data length before submitting a
CAN frame, and queue device output so a temporarily busy serial endpoint does
not corrupt or interleave lines.
