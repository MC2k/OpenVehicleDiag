use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serialport::SerialPort;

use super::comm_api::{
    CanFrame, Capability, ComServer, ComServerError, DeviceCapabilities, FilterType, ISO15765Data,
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(1);
const SERIAL_BAUD: u32 = 115_200;

#[derive(Debug, Clone)]
pub struct SlcanApi {
    port_name: String,
    inner: Arc<Mutex<SlcanInner>>,
}

struct SlcanInner {
    port: Option<Box<dyn SerialPort>>,
    receive_buffer: Vec<u8>,
    receive_queue: VecDeque<CanFrame>,
    iso_receive_queue: VecDeque<CanFrame>,
    can_open: bool,
    iso_tp_open: bool,
    filters: [Option<FilterType>; 10],
    iso_tp_filter: Option<IsoTpFilter>,
    iso_tp_receive: Option<IsoTpReceive>,
    iso_tp_block_size: u8,
    iso_tp_separation_time_min: u8,
}

#[derive(Clone, Copy)]
struct IsoTpFilter {
    response_id: u32,
    mask: u32,
    flow_control_id: u32,
    block_size: u8,
    separation_time_min: u8,
}

struct IsoTpReceive {
    id: u32,
    length: usize,
    data: Vec<u8>,
    next_sequence: u8,
    frames_since_flow_control: u8,
}

impl std::fmt::Debug for SlcanInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlcanInner")
            .field("can_open", &self.can_open)
            .field("queued_frames", &self.receive_queue.len())
            .finish()
    }
}

#[derive(Debug)]
enum SlcanEvent {
    Ack,
    TxAccepted,
    Nack,
    Frame(CanFrame),
    Information,
}

impl SlcanApi {
    pub fn new(port_name: String) -> Self {
        Self {
            port_name,
            inner: Arc::new(Mutex::new(SlcanInner {
                port: None,
                receive_buffer: Vec::new(),
                receive_queue: VecDeque::new(),
                iso_receive_queue: VecDeque::new(),
                can_open: false,
                iso_tp_open: false,
                filters: [None; 10],
                iso_tp_filter: None,
                iso_tp_receive: None,
                iso_tp_block_size: 8,
                iso_tp_separation_time_min: 20,
            })),
        }
    }

    fn error(code: u32, description: impl Into<String>) -> ComServerError {
        ComServerError {
            err_code: code,
            err_desc: description.into(),
        }
    }

    fn bitrate_code(bitrate: u32) -> Option<char> {
        match bitrate {
            10_000 => Some('0'),
            20_000 => Some('1'),
            50_000 => Some('2'),
            100_000 => Some('3'),
            125_000 => Some('4'),
            250_000 => Some('5'),
            500_000 => Some('6'),
            800_000 => Some('7'),
            1_000_000 => Some('8'),
            _ => None,
        }
    }

    fn send_command<F>(
        inner: &mut SlcanInner,
        command: &str,
        response_matches: F,
    ) -> Result<(), ComServerError>
    where
        F: Fn(&SlcanEvent) -> bool,
    {
        let port = inner
            .port
            .as_mut()
            .ok_or_else(|| Self::error(1, "SLCAN serial port is not open"))?;
        port.write_all(command.as_bytes())
            .and_then(|_| port.write_all(b"\r"))
            .and_then(|_| port.flush())
            .map_err(|error| {
                Self::error(
                    error.raw_os_error().unwrap_or_default() as u32,
                    error.to_string(),
                )
            })?;

        let deadline = Instant::now() + COMMAND_TIMEOUT;
        while Instant::now() < deadline {
            for event in Self::poll_events(inner)? {
                match event {
                    event if response_matches(&event) => return Ok(()),
                    SlcanEvent::Nack => {
                        return Err(Self::error(
                            2,
                            format!("SLCAN command '{command}' was rejected"),
                        ));
                    }
                    _ => {}
                }
            }
        }
        Err(Self::error(
            3,
            format!("Timed out waiting for SLCAN response to '{command}'"),
        ))
    }

    fn poll_events(inner: &mut SlcanInner) -> Result<Vec<SlcanEvent>, ComServerError> {
        let mut bytes = [0u8; 256];
        let read_result = {
            let port = inner
                .port
                .as_mut()
                .ok_or_else(|| Self::error(1, "SLCAN serial port is not open"))?;
            port.read(&mut bytes)
        };
        match read_result {
            Ok(count) => inner.receive_buffer.extend_from_slice(&bytes[..count]),
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {}
            Err(error) => {
                return Err(Self::error(
                    error.raw_os_error().unwrap_or_default() as u32,
                    error.to_string(),
                ));
            }
        }

        let mut events = Vec::new();
        let mut line_start = 0;
        let mut index = 0;
        while index < inner.receive_buffer.len() {
            if inner.receive_buffer[index] == 0x07 {
                if index > line_start {
                    events.push(Self::parse_line(&inner.receive_buffer[line_start..index]));
                }
                events.push(SlcanEvent::Nack);
                line_start = index + 1;
            } else if inner.receive_buffer[index] == b'\r' {
                events.push(Self::parse_line(&inner.receive_buffer[line_start..index]));
                line_start = index + 1;
            }
            index += 1;
        }
        inner.receive_buffer.drain(..line_start);

        for event in &events {
            if let SlcanEvent::Frame(frame) = event {
                if inner.iso_tp_open {
                    inner.iso_receive_queue.push_back(*frame);
                }
                if Self::frame_is_allowed(frame, &inner.filters) {
                    inner.receive_queue.push_back(*frame);
                }
            }
        }
        Ok(events)
    }

    fn drain_can_queue(inner: &mut SlcanInner, max_msgs: usize) -> Vec<CanFrame> {
        let count = max_msgs.min(inner.receive_queue.len());
        inner.receive_queue.drain(..count).collect()
    }

    fn parse_line(line: &[u8]) -> SlcanEvent {
        if line.is_empty() {
            return SlcanEvent::Ack;
        }
        match line[0] {
            b't' | b'T' => {
                Self::parse_data_frame(line).map_or(SlcanEvent::Information, SlcanEvent::Frame)
            }
            b'z' if line.len() == 1 => SlcanEvent::TxAccepted,
            _ => SlcanEvent::Information,
        }
    }

    fn parse_data_frame(line: &[u8]) -> Option<CanFrame> {
        let id_width = if line[0] == b't' { 3 } else { 8 };
        if line.len() < 1 + id_width + 1 {
            return None;
        }
        let id = Self::parse_hex(&line[1..1 + id_width])?;
        let max_id = if id_width == 3 { 0x7ff } else { 0x1fff_ffff };
        if id > max_id {
            return None;
        }
        let dlc = Self::parse_hex(&line[1 + id_width..2 + id_width])?;
        if dlc > 8 {
            return None;
        }
        let data_end = 2 + id_width + dlc as usize * 2;
        if line.len() != data_end && line.len() != data_end + 4 {
            return None;
        }
        let mut data = [0u8; 8];
        for (index, byte) in data[..dlc as usize].iter_mut().enumerate() {
            *byte =
                Self::parse_hex(&line[2 + id_width + index * 2..4 + id_width + index * 2])? as u8;
        }
        Some(CanFrame::new(id, &data[..dlc as usize]))
    }

    fn parse_hex(value: &[u8]) -> Option<u32> {
        std::str::from_utf8(value)
            .ok()
            .and_then(|value| u32::from_str_radix(value, 16).ok())
    }

    fn frame_is_allowed(frame: &CanFrame, filters: &[Option<FilterType>; 10]) -> bool {
        let matches = |id, mask| frame.id & mask == id & mask;
        if filters.iter().flatten().any(|filter| match filter {
            FilterType::Block { id, mask } => matches(*id, *mask),
            _ => false,
        }) {
            return false;
        }
        let has_pass_filter = filters
            .iter()
            .flatten()
            .any(|filter| matches!(filter, FilterType::Pass { .. }));
        !has_pass_filter
            || filters.iter().flatten().any(|filter| match filter {
                FilterType::Pass { id, mask } => matches(*id, *mask),
                _ => false,
            })
    }

    fn encode_frame(frame: &CanFrame) -> Result<String, ComServerError> {
        if frame.id <= 0x7ff {
            Ok(format!(
                "t{:03X}{:X}{}",
                frame.id,
                frame.dlc,
                hex::encode_upper(frame.get_data())
            ))
        } else if frame.id <= 0x1fff_ffff {
            Ok(format!(
                "T{:08X}{:X}{}",
                frame.id,
                frame.dlc,
                hex::encode_upper(frame.get_data())
            ))
        } else {
            Err(Self::error(
                4,
                format!("CAN ID 0x{:X} is invalid", frame.id),
            ))
        }
    }

    fn send_frame(inner: &mut SlcanInner, frame: &CanFrame) -> Result<(), ComServerError> {
        let command = Self::encode_frame(frame)?;
        Self::send_command(inner, &command, |event| {
            matches!(event, SlcanEvent::TxAccepted)
        })
    }

    fn send_iso_tp_flow_control(
        inner: &mut SlcanInner,
        filter: IsoTpFilter,
    ) -> Result<(), ComServerError> {
        let data = [
            0x30,
            filter.block_size,
            filter.separation_time_min,
            0,
            0,
            0,
            0,
            0,
        ];
        Self::send_frame(inner, &CanFrame::new(filter.flow_control_id, &data))
    }

    fn receive_flow_control(
        inner: &mut SlcanInner,
        response_id: u32,
        mask: u32,
        deadline: Instant,
    ) -> Result<(u8, Duration), ComServerError> {
        loop {
            Self::poll_events(inner)?;
            while let Some(frame) = inner.iso_receive_queue.pop_front() {
                if frame.id & mask != response_id & mask {
                    continue;
                }
                let data = frame.get_data();
                if data.len() < 3 || data[0] >> 4 != 3 {
                    continue;
                }
                match data[0] & 0x0f {
                    0 => {
                        let delay = match data[2] {
                            value @ 0..=0x7f => Duration::from_millis(value as u64),
                            value @ 0xf1..=0xf9 => {
                                Duration::from_micros((value as u64 - 0xf0) * 100)
                            }
                            _ => {
                                return Err(Self::error(
                                    11,
                                    "ECU sent an invalid ISO-TP separation time",
                                ))
                            }
                        };
                        return Ok((data[1], delay));
                    }
                    1 => {}
                    2 => return Err(Self::error(12, "ECU rejected the ISO-TP transfer")),
                    _ => {
                        return Err(Self::error(
                            13,
                            "ECU sent an invalid ISO-TP flow-control frame",
                        ))
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(Self::error(14, "Timed out waiting for ISO-TP flow control"));
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn send_iso_tp_data(
        inner: &mut SlcanInner,
        data: &ISO15765Data,
        filter: IsoTpFilter,
    ) -> Result<(), ComServerError> {
        if data.ext_addressing {
            return Err(Self::error(
                15,
                "Extended ISO-TP addressing is not supported by SLCAN",
            ));
        }
        if data.data.len() > 0x0fff {
            return Err(Self::error(16, "SLCAN ISO-TP payload exceeds 4095 bytes"));
        }
        if data.data.len() <= 7 {
            let mut frame_data = Vec::with_capacity(if data.pad_frame {
                8
            } else {
                data.data.len() + 1
            });
            frame_data.push(data.data.len() as u8);
            frame_data.extend_from_slice(&data.data);
            if data.pad_frame {
                frame_data.resize(8, 0);
            }
            return Self::send_frame(inner, &CanFrame::new(data.id, &frame_data));
        }

        let length = data.data.len();
        let mut first_frame = Vec::with_capacity(8);
        first_frame.push(0x10 | ((length >> 8) as u8 & 0x0f));
        first_frame.push(length as u8);
        first_frame.extend_from_slice(&data.data[..6]);
        Self::send_frame(inner, &CanFrame::new(data.id, &first_frame))?;

        let (mut block_size, separation_time) = Self::receive_flow_control(
            inner,
            filter.response_id,
            filter.mask,
            Instant::now() + COMMAND_TIMEOUT,
        )?;
        let mut offset = 6;
        let mut sequence = 1u8;
        let mut frames_in_block = 0u8;
        while offset < length {
            let end = (offset + 7).min(length);
            let mut consecutive_frame = Vec::with_capacity(8);
            consecutive_frame.push(0x20 | sequence);
            consecutive_frame.extend_from_slice(&data.data[offset..end]);
            if data.pad_frame {
                consecutive_frame.resize(8, 0);
            }
            Self::send_frame(inner, &CanFrame::new(data.id, &consecutive_frame))?;
            offset = end;
            sequence = (sequence + 1) & 0x0f;
            frames_in_block += 1;
            if !separation_time.is_zero() && offset < length {
                thread::sleep(separation_time);
            }
            if block_size != 0 && frames_in_block >= block_size && offset < length {
                let flow_control = Self::receive_flow_control(
                    inner,
                    filter.response_id,
                    filter.mask,
                    Instant::now() + COMMAND_TIMEOUT,
                )?;
                block_size = flow_control.0;
                frames_in_block = 0;
            }
        }
        Ok(())
    }

    fn receive_iso_tp_frame(
        inner: &mut SlcanInner,
        frame: CanFrame,
    ) -> Result<Option<ISO15765Data>, ComServerError> {
        let filter = match inner.iso_tp_filter {
            Some(filter) if frame.id & filter.mask == filter.response_id & filter.mask => filter,
            _ => return Ok(None),
        };
        let bytes = frame.get_data();
        if bytes.is_empty() {
            return Ok(None);
        }
        match bytes[0] >> 4 {
            0 => {
                let length = (bytes[0] & 0x0f) as usize;
                if bytes.len() < length + 1 {
                    return Err(Self::error(
                        17,
                        "ISO-TP single frame is shorter than its declared length",
                    ));
                }
                Ok(Some(ISO15765Data {
                    id: frame.id,
                    data: bytes[1..1 + length].to_vec(),
                    pad_frame: false,
                    ext_addressing: false,
                }))
            }
            1 => {
                if bytes.len() < 2 {
                    return Err(Self::error(18, "ISO-TP first frame is truncated"));
                }
                let length = (((bytes[0] & 0x0f) as usize) << 8) | bytes[1] as usize;
                if length <= 7 || bytes.len() < 3 {
                    return Err(Self::error(19, "ISO-TP first frame has an invalid length"));
                }
                let mut data = bytes[2..].to_vec();
                data.truncate(length);
                inner.iso_tp_receive = Some(IsoTpReceive {
                    id: frame.id,
                    length,
                    data,
                    next_sequence: 1,
                    frames_since_flow_control: 0,
                });
                Self::send_iso_tp_flow_control(inner, filter)?;
                Ok(None)
            }
            2 => {
                let receive = match inner.iso_tp_receive.as_mut() {
                    Some(receive) if receive.id == frame.id => receive,
                    _ => return Ok(None),
                };
                if bytes[0] & 0x0f != receive.next_sequence {
                    inner.iso_tp_receive = None;
                    return Err(Self::error(
                        20,
                        "ISO-TP consecutive-frame sequence mismatch",
                    ));
                }
                receive.data.extend_from_slice(&bytes[1..]);
                receive.data.truncate(receive.length);
                receive.next_sequence = (receive.next_sequence + 1) & 0x0f;
                receive.frames_since_flow_control += 1;
                let completed = receive.data.len() == receive.length;
                let send_flow_control = !completed
                    && filter.block_size != 0
                    && receive.frames_since_flow_control >= filter.block_size;
                if completed {
                    let receive = inner.iso_tp_receive.take().unwrap();
                    return Ok(Some(ISO15765Data {
                        id: receive.id,
                        data: receive.data,
                        pad_frame: false,
                        ext_addressing: false,
                    }));
                }
                if send_flow_control {
                    inner
                        .iso_tp_receive
                        .as_mut()
                        .unwrap()
                        .frames_since_flow_control = 0;
                    Self::send_iso_tp_flow_control(inner, filter)?;
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

impl ComServer for SlcanApi {
    fn open_device(&mut self) -> Result<(), ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.port.is_some() {
            return Ok(());
        }
        inner.port = Some(
            serialport::new(&self.port_name, SERIAL_BAUD)
                .timeout(Duration::from_millis(10))
                .open()
                .map_err(|error| Self::error(1, error.to_string()))?,
        );
        Ok(())
    }

    fn close_device(&mut self) -> Result<(), ComServerError> {
        self.close_can_interface()?;
        let mut inner = self.inner.lock().unwrap();
        inner.port.take();
        inner.receive_buffer.clear();
        inner.receive_queue.clear();
        Ok(())
    }

    fn send_can_packets(
        &mut self,
        data: &[CanFrame],
        _timeout_ms: u32,
    ) -> Result<usize, ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.can_open {
            return Err(Self::error(5, "SLCAN CAN channel is not open"));
        }
        for frame in data {
            Self::send_frame(&mut inner, frame)?;
        }
        Ok(data.len())
    }

    fn read_can_packets(
        &self,
        timeout_ms: u32,
        max_msgs: usize,
    ) -> Result<Vec<CanFrame>, ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.can_open {
            return Err(Self::error(5, "SLCAN CAN channel is not open"));
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
        loop {
            Self::poll_events(&mut inner)?;
            if !inner.receive_queue.is_empty() || timeout_ms == 0 || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(Self::drain_can_queue(&mut inner, max_msgs))
    }

    fn send_iso15765_data(
        &self,
        data: &[ISO15765Data],
        _timeout_ms: u32,
    ) -> Result<usize, ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        let filter = inner
            .iso_tp_filter
            .ok_or_else(|| Self::error(21, "SLCAN ISO-TP filter is not configured"))?;
        if !inner.iso_tp_open {
            return Err(Self::error(6, "SLCAN ISO-TP interface is not open"));
        }
        for payload in data {
            Self::send_iso_tp_data(&mut inner, payload, filter)?;
        }
        Ok(data.len())
    }

    fn read_iso15765_packets(
        &self,
        timeout_ms: u32,
        max_msgs: usize,
    ) -> Result<Vec<ISO15765Data>, ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.iso_tp_open {
            return Err(Self::error(6, "SLCAN ISO-TP interface is not open"));
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
        let mut payloads = Vec::with_capacity(max_msgs);
        loop {
            Self::poll_events(&mut inner)?;
            while let Some(frame) = inner.iso_receive_queue.pop_front() {
                if let Some(payload) = Self::receive_iso_tp_frame(&mut inner, frame)? {
                    payloads.push(payload);
                    if payloads.len() == max_msgs {
                        return Ok(payloads);
                    }
                }
            }
            if timeout_ms == 0 || Instant::now() >= deadline {
                return Ok(payloads);
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn open_can_interface(
        &mut self,
        bus_speed: u32,
        _is_ext_can: bool,
    ) -> Result<(), ComServerError> {
        let code = Self::bitrate_code(bus_speed)
            .ok_or_else(|| Self::error(7, format!("SLCAN does not support {bus_speed} bit/s")))?;
        let mut inner = self.inner.lock().unwrap();
        if inner.can_open {
            Self::send_command(&mut inner, "C", |event| matches!(event, SlcanEvent::Ack))?;
            inner.can_open = false;
        } else {
            // A prior client may have disconnected without closing the CAN controller.
            // NACK means the controller was already closed, which is the desired state.
            Self::send_command(&mut inner, "C", |event| {
                matches!(event, SlcanEvent::Ack | SlcanEvent::Nack)
            })?;
        }
        inner.iso_tp_open = false;
        inner.iso_tp_filter = None;
        inner.iso_tp_receive = None;
        inner.iso_receive_queue.clear();
        Self::send_command(&mut inner, &format!("S{code}"), |event| {
            matches!(event, SlcanEvent::Ack)
        })?;
        Self::send_command(&mut inner, "O", |event| matches!(event, SlcanEvent::Ack))?;
        inner.can_open = true;
        Ok(())
    }

    fn close_can_interface(&mut self) -> Result<(), ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.can_open && inner.port.is_some() {
            Self::send_command(&mut inner, "C", |event| matches!(event, SlcanEvent::Ack))?;
        }
        inner.can_open = false;
        inner.iso_tp_open = false;
        inner.filters = [None; 10];
        inner.iso_tp_filter = None;
        inner.iso_tp_receive = None;
        inner.receive_queue.clear();
        inner.iso_receive_queue.clear();
        Ok(())
    }

    fn open_iso15765_interface(
        &mut self,
        bus_speed: u32,
        is_ext_can: bool,
        ext_addressing: bool,
    ) -> Result<(), ComServerError> {
        if ext_addressing {
            return Err(Self::error(
                15,
                "Extended ISO-TP addressing is not supported by SLCAN",
            ));
        }
        self.open_can_interface(bus_speed, is_ext_can)?;
        let mut inner = self.inner.lock().unwrap();
        inner.iso_tp_open = true;
        inner.iso_tp_receive = None;
        inner.iso_receive_queue.clear();
        Ok(())
    }

    fn close_iso15765_interface(&mut self) -> Result<(), ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        inner.iso_tp_open = false;
        inner.iso_tp_filter = None;
        inner.iso_tp_receive = None;
        inner.iso_receive_queue.clear();
        Ok(())
    }

    fn add_can_filter(&mut self, filter: FilterType) -> Result<u32, ComServerError> {
        if matches!(filter, FilterType::IsoTP { .. }) {
            return Err(Self::error(8, "Cannot apply an ISO-TP filter to SLCAN CAN"));
        }
        let mut inner = self.inner.lock().unwrap();
        let index = inner
            .filters
            .iter()
            .position(Option::is_none)
            .ok_or_else(|| Self::error(9, "No free SLCAN CAN filters"))?;
        inner.filters[index] = Some(filter);
        Ok(index as u32)
    }

    fn rem_can_filter(&mut self, filter_idx: u32) -> Result<(), ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        let filter = inner
            .filters
            .get_mut(filter_idx as usize)
            .ok_or_else(|| Self::error(10, "Invalid SLCAN CAN filter index"))?;
        *filter = None;
        Ok(())
    }

    fn add_iso15765_filter(&mut self, filter: FilterType) -> Result<u32, ComServerError> {
        let FilterType::IsoTP { id, mask, fc } = filter else {
            return Err(Self::error(
                22,
                "SLCAN ISO-TP requires a flow-control filter",
            ));
        };
        let mut inner = self.inner.lock().unwrap();
        if !inner.iso_tp_open {
            return Err(Self::error(6, "SLCAN ISO-TP interface is not open"));
        }
        inner.iso_tp_filter = Some(IsoTpFilter {
            response_id: id,
            mask,
            flow_control_id: fc,
            block_size: inner.iso_tp_block_size,
            separation_time_min: inner.iso_tp_separation_time_min,
        });
        Ok(0)
    }

    fn rem_iso15765_filter(&mut self, filter_idx: u32) -> Result<(), ComServerError> {
        if filter_idx != 0 {
            return Err(Self::error(23, "Invalid SLCAN ISO-TP filter index"));
        }
        self.inner.lock().unwrap().iso_tp_filter = None;
        Ok(())
    }

    fn set_iso15765_params(
        &mut self,
        separation_time_min: u32,
        block_size: u32,
    ) -> Result<(), ComServerError> {
        if separation_time_min > 0x7f || block_size > u8::MAX as u32 {
            return Err(Self::error(
                24,
                "SLCAN ISO-TP flow-control parameters are invalid",
            ));
        }
        let mut inner = self.inner.lock().unwrap();
        if !inner.iso_tp_open {
            return Err(Self::error(6, "SLCAN ISO-TP interface is not open"));
        }
        inner.iso_tp_separation_time_min = separation_time_min as u8;
        inner.iso_tp_block_size = block_size as u8;
        let separation_time_min = inner.iso_tp_separation_time_min;
        let block_size = inner.iso_tp_block_size;
        if let Some(filter) = inner.iso_tp_filter.as_mut() {
            filter.separation_time_min = separation_time_min;
            filter.block_size = block_size;
        }
        Ok(())
    }

    fn clear_can_rx_buffer(&self) -> Result<(), ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        Self::poll_events(&mut inner)?;
        inner.receive_queue.clear();
        Ok(())
    }

    fn clear_can_tx_buffer(&self) -> Result<(), ComServerError> {
        Ok(())
    }

    fn clear_iso15765_rx_buffer(&self) -> Result<(), ComServerError> {
        let mut inner = self.inner.lock().unwrap();
        Self::poll_events(&mut inner)?;
        inner.iso_receive_queue.clear();
        inner.iso_tp_receive = None;
        Ok(())
    }

    fn clear_iso15765_tx_buffer(&self) -> Result<(), ComServerError> {
        Ok(())
    }

    fn read_battery_voltage(&self) -> Result<f32, ComServerError> {
        Ok(-1.0)
    }

    fn clone_box(&self) -> Box<dyn ComServer> {
        Box::new(self.clone())
    }

    fn get_capabilities(&self) -> DeviceCapabilities {
        DeviceCapabilities {
            name: self.port_name.clone(),
            vendor: "SLCAN".into(),
            library_path: "Serial port".into(),
            device_fw_version: "Unknown".into(),
            library_version: "SLCAN".into(),
            j1850vpw: Capability::NA,
            j1850pwm: Capability::NA,
            can: Capability::Yes,
            iso15765: Capability::Yes,
            iso9141: Capability::NA,
            iso14230: Capability::NA,
            ip: Capability::NA,
            battery_voltage: Capability::NA,
        }
    }

    fn get_api(&self) -> &str {
        "SLCAN"
    }

    fn is_connected(&self) -> bool {
        self.inner.lock().unwrap().port.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{IsoTpFilter, SlcanApi, SlcanInner};
    use crate::commapi::comm_api::CanFrame;
    use std::collections::VecDeque;

    #[test]
    fn drains_can_queue_up_to_available_frames_and_requested_limit() {
        for (queued, max_msgs, expected) in [
            (0, 100, 0),
            (3, 100, 3), // CAN Analyzer crash: requested 100 with only 3 available.
            (100, 100, 100),
            (103, 100, 100),
            (3, 0, 0),
        ] {
            let api = SlcanApi::new(String::new());
            let mut inner = api.inner.lock().unwrap();
            inner.receive_queue.extend(
                (0..queued).map(|id| CanFrame::new(id as u32, &[id as u8])),
            );

            let frames = SlcanApi::drain_can_queue(&mut inner, max_msgs);
            assert_eq!(frames.len(), expected);
            for (id, frame) in frames.iter().enumerate() {
                assert_eq!(frame.id, id as u32);
                assert_eq!(frame.get_data(), &[id as u8]);
            }
            assert_eq!(inner.receive_queue.len(), queued - expected);

            let remaining = SlcanApi::drain_can_queue(&mut inner, usize::MAX);
            assert_eq!(remaining.len(), queued - expected);
            for (id, frame) in (expected..queued).zip(remaining) {
                assert_eq!(frame.id, id as u32);
                assert_eq!(frame.get_data(), &[id as u8]);
            }
            assert!(inner.receive_queue.is_empty());
        }
    }

    #[test]
    fn parses_standard_frame_with_timestamp() {
        let frame = SlcanApi::parse_data_frame(b"t1232A1B2ABCD").unwrap();
        assert_eq!(frame.id, 0x123);
        assert_eq!(frame.get_data(), &[0xa1, 0xb2]);
    }

    #[test]
    fn parses_extended_frame_without_timestamp() {
        let frame = SlcanApi::parse_data_frame(b"T1ABCDEFF3DEADBE").unwrap();
        assert_eq!(frame.id, 0x1abc_deff);
        assert_eq!(frame.get_data(), &[0xde, 0xad, 0xbe]);
    }

    #[test]
    fn reassembles_iso_tp_single_frame() {
        let mut inner = SlcanInner {
            port: None,
            receive_buffer: Vec::new(),
            receive_queue: VecDeque::new(),
            iso_receive_queue: VecDeque::new(),
            can_open: true,
            iso_tp_open: true,
            filters: [None; 10],
            iso_tp_filter: Some(IsoTpFilter {
                response_id: 0x7e8,
                mask: 0x7ff,
                flow_control_id: 0x7e0,
                block_size: 8,
                separation_time_min: 20,
            }),
            iso_tp_receive: None,
            iso_tp_block_size: 8,
            iso_tp_separation_time_min: 20,
        };
        let payload = SlcanApi::receive_iso_tp_frame(
            &mut inner,
            CanFrame::new(0x7e8, &[0x03, 0x41, 0x00, 0xff]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(payload.id, 0x7e8);
        assert_eq!(payload.data, &[0x41, 0x00, 0xff]);
    }
}
