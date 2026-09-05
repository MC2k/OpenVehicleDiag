use std::{fs::File, io::Read};

use common::schema::{ConType, Connection, OvdECU};
use iced::{Align, Column, Element, Length, Row, Scrollable, Subscription};

use crate::{
    commapi::comm_api::{Capability, ComServer, ISO15765Config},
    themes::{
        button_outlined, picklist, radio_btn, text, text_input, title_text, ButtonType, TextType,
        TitleSize,
    },
};

use super::{
    diag_home::{ECUDiagSettings, VehicleECUList},
    diag_session::{DiagMessageTrait, DiagSession, SessionMsg, SessionType},
};

#[derive(Debug, Clone)]
pub enum DiagManualMessage {
    LaunchFileBrowser,
    PickECU(ECUDiagSettings),
    PickConnection(usize),
    LaunchJSON,
    LaunchKWP,
    LaunchKWPCustom,
    LaunchUDS,
    LaunchUDSCustom,
    LaunchCustom,
    LaunchCustomCustom,
    Session(SessionMsg),

    //User input queues
    SendIDEnter(String),
    RecvIDEnter(String),
    SepEnter(String),
    BsEnter(String),
}

#[derive(Debug, Clone)]
pub struct DiagManual {
    server: Box<dyn ComServer>,
    car: Option<VehicleECUList>,
    btn_state: iced::button::State,
    pick_state: iced::pick_list::State<ECUDiagSettings>,
    status: String,
    curr_ecu: Option<ECUDiagSettings>,
    uds_btn_state: iced::button::State,
    kwp_btn_state: iced::button::State,
    custom_btn_state: iced::button::State,
    json_btn_state: iced::button::State,
    json_ecu: Option<OvdECU>,
    json_connection: Option<usize>,
    scroll_state: iced::scrollable::State,
    session: Option<DiagSession>,

    // Input for custom session!
    str_send_id: String,
    str_recv_id: String,
    str_bs: String,
    str_sep: String,

    input_send_id: iced::text_input::State,
    input_recv_id: iced::text_input::State,
    input_bs: iced::text_input::State,
    input_sep: iced::text_input::State,

    uds_btn_state_2: iced::button::State,
    kwp_btn_state_2: iced::button::State,
    custom_btn_state_2: iced::button::State,
}

impl DiagManual {
    pub(crate) fn new(server: Box<dyn ComServer>) -> Self {
        Self {
            server,
            car: None,
            btn_state: Default::default(),
            pick_state: Default::default(),
            status: "".into(),
            curr_ecu: None,
            uds_btn_state: Default::default(),
            kwp_btn_state: Default::default(),
            custom_btn_state: Default::default(),
            json_btn_state: Default::default(),
            json_ecu: None,
            json_connection: None,
            scroll_state: Default::default(),
            session: None,
            str_send_id: Default::default(),
            str_recv_id: Default::default(),
            str_bs: Default::default(),
            str_sep: Default::default(),
            input_send_id: Default::default(),
            input_recv_id: Default::default(),
            input_bs: Default::default(),
            input_sep: Default::default(),
            uds_btn_state_2: Default::default(),
            kwp_btn_state_2: Default::default(),
            custom_btn_state_2: Default::default(),
        }
    }

    pub fn subscription(&self) -> Subscription<DiagManualMessage> {
        if let Some(ref session) = self.session {
            session.subscription().map(DiagManualMessage::Session)
        } else {
            Subscription::none()
        }
    }

    pub fn update(&mut self, msg: &DiagManualMessage) -> Option<DiagManualMessage> {
        // If session is active, all calls get re-directed to the active diag session
        if let Some(ref mut session) = self.session {
            if let DiagManualMessage::Session(m) = msg {
                if m.is_back() {
                    self.session.take();
                    return None;
                } else {
                    return session.update(m).map(DiagManualMessage::Session);
                }
            }
        }
        match msg {
            DiagManualMessage::LaunchFileBrowser => {
                if let nfd::Response::Okay(f_path) =
                    nfd::open_file_dialog(Some("json"), None).unwrap_or(nfd::Response::Cancel)
                {
                    if let Ok(mut file) = File::open(f_path) {
                        let mut str = "".into();
                        if file.read_to_string(&mut str).is_ok() {
                            if let Ok(car) = serde_json::from_str::<VehicleECUList>(&str) {
                                println!("Car save opened!");
                                self.curr_ecu = None;
                                self.json_ecu = None;
                                self.json_connection = None;
                                self.status.clear();
                                self.car = Some(car)
                            } else if let Ok(ecu) = serde_json::from_str::<OvdECU>(&str) {
                                self.load_json_ecu(ecu);
                            } else {
                                self.status = format!("Error processing input file!")
                            }
                        } else {
                            self.status = format!("Error reading input file!")
                        }
                    } else {
                        self.status = "Error loading save file".into()
                    }
                }
            }
            DiagManualMessage::PickECU(e) => self.curr_ecu = Some(e.clone()),
            DiagManualMessage::PickConnection(index) => {
                self.json_connection = self
                    .json_ecu
                    .as_ref()
                    .and_then(|ecu| ecu.connections.get(*index).map(|_| *index));
                self.status.clear();
            }
            DiagManualMessage::LaunchJSON => self.launch_json_session(),
            DiagManualMessage::LaunchKWP => self.launch_diag_session(SessionType::KWP, false),
            DiagManualMessage::LaunchUDS => self.launch_diag_session(SessionType::UDS, false),
            DiagManualMessage::LaunchCustom => self.launch_diag_session(SessionType::Custom, false),
            DiagManualMessage::LaunchKWPCustom => self.launch_diag_session(SessionType::KWP, true),
            DiagManualMessage::LaunchUDSCustom => self.launch_diag_session(SessionType::UDS, true),
            DiagManualMessage::LaunchCustomCustom => {
                self.launch_diag_session(SessionType::Custom, true)
            }
            DiagManualMessage::BsEnter(s) => {
                if s.is_empty() {
                    self.status.clear();
                    self.str_bs.clear();
                } else {
                    match s.parse::<u32>() {
                        Ok(_) => {
                            self.status.clear();
                            self.str_bs = s.clone();
                        }
                        Err(_) => self.status = format!("{} is not a number", s),
                    }
                }
            }
            DiagManualMessage::SendIDEnter(s) => {
                if s.is_empty() {
                    self.status.clear();
                    self.str_send_id.clear();
                } else {
                    match hex::decode(s) {
                        Err(e) if e == hex::FromHexError::OddLength => {
                            self.status.clear();
                            self.str_send_id = s.clone();
                            self.status = "Require even number of characters".into()
                        }
                        Ok(_) => {
                            self.status.clear();
                            self.str_send_id = s.clone();
                        }
                        Err(_) => self.status = format!("{} is not a hex number", s),
                    }
                }
            }
            DiagManualMessage::RecvIDEnter(s) => {
                if s.is_empty() {
                    self.status.clear();
                    self.str_recv_id.clear();
                } else {
                    match hex::decode(s) {
                        Err(e) if e == hex::FromHexError::OddLength => {
                            self.status.clear();
                            self.str_recv_id = s.clone();
                            self.status = "Require even number of characters".into()
                        }
                        Ok(_) => {
                            self.status.clear();
                            self.str_recv_id = s.clone();
                        }
                        Err(_) => self.status = format!("{} is not a hex number", s),
                    }
                }
            }
            DiagManualMessage::SepEnter(s) => {
                if s.is_empty() {
                    self.status.clear();
                    self.str_sep.clear();
                } else {
                    match s.parse::<u32>() {
                        Ok(_) => {
                            self.status.clear();
                            self.str_sep = s.clone();
                        }
                        Err(_) => self.status = format!("{} is not a number", s),
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn load_json_ecu(&mut self, ecu: OvdECU) {
        let connection_count = ecu.connections.len();
        self.car = None;
        self.curr_ecu = None;
        self.json_ecu = Some(ecu);
        self.json_connection = None;
        self.status.clear();
        match connection_count {
            0 => self.status = "ECU JSON file has no connections".into(),
            1 => {
                self.json_connection = Some(0);
                self.launch_json_session();
            }
            _ => {}
        }
    }

    fn json_connection_error(connection: &Connection, iso_tp: Capability) -> Option<&'static str> {
        match connection.connection_type {
            ConType::LIN { .. } => Some("K-Line is not implemented at this time"),
            ConType::ISOTP { .. } if iso_tp != Capability::Yes => {
                Some("Selected adapter does not support ISO-TP over CAN")
            }
            _ => None,
        }
    }

    fn launch_json_session(&mut self) {
        let selected = self.json_ecu.as_ref().and_then(|ecu| {
            self.json_connection
                .and_then(|index| ecu.connections.get(index))
                .map(|connection| (ecu, connection))
        });
        let (ecu, connection) = match selected {
            Some(selected) => selected,
            None => {
                self.status = "Select an ECU JSON connection first".into();
                return;
            }
        };
        if let Some(error) = Self::json_connection_error(
            connection,
            self.server.get_capabilities().supports_iso15765(),
        ) {
            self.status = error.into();
            return;
        }
        // Keep the definition and selection available if initialization fails or the user returns.
        let session_type = SessionType::JSON(ecu.clone(), connection.clone());
        self.status.clear();
        self.launch_diag_session(session_type, false);
    }

    fn decode_string_hex(s: &str) -> Option<u32> {
        if s.is_empty() {
            return None;
        }
        match hex::decode(s) {
            Ok(res) => {
                if res.len() == 1 {
                    Some(res[0] as u32)
                } else if res.len() == 2 {
                    Some(((res[0] as u16) << 8 | res[1] as u16) as u32)
                } else if res.len() >= 4 {
                    // 4 or more
                    Some(
                        (res[0] as u32) << 24
                            | (res[1] as u32) << 16
                            | (res[2] as u32) << 8
                            | (res[3] as u32),
                    )
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    fn decode_string_int(s: &str) -> Option<u32> {
        match s.parse::<u32>() {
            Ok(i) => Some(i),
            Err(_) => None,
        }
    }

    pub fn launch_diag_session(&mut self, session_type: SessionType, use_custom: bool) {
        if self.session.is_some() {
            self.status = "Error. Diagnostic session already in progress??".into(); // How did this happen??
            return;
        }

        if let SessionType::JSON(_, _) = &session_type {
            match DiagSession::new(&session_type, self.server.clone(), None) {
                Ok(session) => self.session = Some(session),
                Err(e) => self.status = format!("Error init diag session: {}", e.get_description()),
            }
        } else if use_custom {
            let cfg = ISO15765Config {
                baud: 500000,
                send_id: Self::decode_string_hex(&self.str_send_id).unwrap(),
                recv_id: Self::decode_string_hex(&self.str_recv_id).unwrap(),
                block_size: Self::decode_string_int(&self.str_bs).unwrap(),
                sep_time: Self::decode_string_int(&self.str_sep).unwrap(),
                use_ext_isotp: false,
                use_ext_can: false,
            };
            match DiagSession::new(&session_type, self.server.clone(), Some(cfg)) {
                Ok(session) => self.session = Some(session),
                Err(e) => self.status = format!("Error init diag session: {}", e.get_description()),
            }
        } else if let Some(ecu) = &self.curr_ecu {
            let cfg = ISO15765Config {
                baud: 500000,
                send_id: ecu.send_id,
                recv_id: ecu.flow_control_id,
                block_size: ecu.block_size,
                sep_time: ecu.sep_time_ms,
                use_ext_isotp: false,
                use_ext_can: false,
            };
            match DiagSession::new(&session_type, self.server.clone(), Some(cfg)) {
                Ok(session) => self.session = Some(session),
                Err(e) => self.status = format!("Error init diag session: {}", e.get_description()),
            }
        } else {
            self.status = "Error. No ECU selected?".into(); // How did this happen??
        }
    }

    pub fn view(&mut self) -> Element<'_, DiagManualMessage> {
        if let Some(ref mut session) = self.session {
            return session.view().map(DiagManualMessage::Session);
        }
        let mut view = Column::new()
            .padding(20)
            .spacing(20)
            .align_items(Align::Center)
            .width(Length::Fill)
            .push(title_text(
                "Load a save file or ECU JSON file to get started",
                TitleSize::P3,
            ));

        view = view.push(
            button_outlined(
                &mut self.btn_state,
                "Load save / ECU JSON file",
                ButtonType::Success,
            )
            .on_press(DiagManualMessage::LaunchFileBrowser),
        );

        if let Some(car) = &self.car {
            view = view.push(text(
                format!(
                    "Loaded car: {} {} ({})",
                    car.vehicle_brand, car.vehicle_name, car.vehicle_year
                )
                .as_str(),
                TextType::Normal,
            ));

            view = view.push(picklist(
                &mut self.pick_state,
                car.ecu_list.clone(),
                self.curr_ecu.clone(),
                DiagManualMessage::PickECU,
            ));

            if let Some(ecu) = &self.curr_ecu {
                let kwp_text = if ecu.kwp_support {
                    "Launch KWP2000 session"
                } else {
                    "KWP2000 not supported"
                };
                let mut kwp_btn =
                    button_outlined(&mut self.kwp_btn_state, kwp_text, ButtonType::Primary)
                        .width(Length::Units(250));
                if ecu.kwp_support {
                    kwp_btn = kwp_btn.on_press(DiagManualMessage::LaunchKWP);
                }

                let uds_text = if ecu.uds_support {
                    "Launch UDS session"
                } else {
                    "UDS not supported"
                };
                let mut uds_btn =
                    button_outlined(&mut self.uds_btn_state, uds_text, ButtonType::Primary)
                        .width(Length::Units(250));
                if ecu.uds_support {
                    uds_btn = uds_btn.on_press(DiagManualMessage::LaunchUDS);
                }

                let custom_btn = button_outlined(
                    &mut self.custom_btn_state,
                    "Launch Custom session",
                    ButtonType::Warning,
                )
                .on_press(DiagManualMessage::LaunchCustom)
                .width(Length::Units(250));

                view = view.push(
                    Row::new()
                        .spacing(8)
                        .push(kwp_btn)
                        .push(uds_btn)
                        .push(custom_btn),
                );
            }
        }

        if let Some(ecu) = &self.json_ecu {
            let iso_tp = self.server.get_capabilities().supports_iso15765();
            let mut connections = Column::new()
                .spacing(8)
                .push(text(&format!("Loaded ECU: {}", ecu.name), TextType::Normal))
                .push(text("Select a diagnostic connection:", TextType::Normal));
            for (index, connection) in ecu.connections.iter().enumerate() {
                let transport = match connection.connection_type {
                    ConType::ISOTP { .. } => "ISO-TP / CAN",
                    ConType::LIN { .. } => "K-Line",
                };
                let mut label = format!(
                    "{}. {:?} over {} - {} bit/s - TX 0x{:X}, RX 0x{:X}",
                    index + 1,
                    connection.server_type,
                    transport,
                    connection.baud,
                    connection.send_id,
                    connection.recv_id,
                );
                if let Some(error) = Self::json_connection_error(connection, iso_tp) {
                    label.push_str(&format!(" ({})", error));
                }
                connections = connections.push(radio_btn(
                    index,
                    label,
                    self.json_connection,
                    DiagManualMessage::PickConnection,
                    ButtonType::Primary,
                ));
            }
            let mut launch = button_outlined(
                &mut self.json_btn_state,
                "Launch JSON session",
                ButtonType::Primary,
            );
            if let Some(connection) = self.json_connection.and_then(|i| ecu.connections.get(i)) {
                if let Some(error) = Self::json_connection_error(connection, iso_tp) {
                    connections = connections.push(text(error, TextType::Warning));
                } else {
                    launch = launch.on_press(DiagManualMessage::LaunchJSON);
                }
            }
            view = view.push(connections.push(launch));
        }

        view = view.push(title_text(
            "Or specify manual ISO-TP Settings",
            TitleSize::P3,
        ));
        view = view.push(
            Row::new()
                .padding(5)
                .spacing(5)
                .width(Length::Fill)
                .align_items(Align::Center) // Row entry for user input
                .push(
                    Column::new()
                        .spacing(2)
                        .width(Length::FillPortion(1)) // First input column
                        .push(text("Send (FC) ID", TextType::Normal))
                        .push(text_input(
                            &mut self.input_send_id,
                            "Enter send ID (Hex)",
                            &self.str_send_id,
                            DiagManualMessage::SendIDEnter,
                        )),
                )
                .push(
                    Column::new()
                        .spacing(2)
                        .width(Length::FillPortion(1)) // Second input column
                        .push(text("Receive ID", TextType::Normal))
                        .push(text_input(
                            &mut self.input_recv_id,
                            "Enter receive ID (Hex)",
                            &self.str_recv_id,
                            DiagManualMessage::RecvIDEnter,
                        )),
                )
                .push(
                    Column::new()
                        .spacing(2)
                        .width(Length::FillPortion(1)) // Third input column
                        .push(text("Separation time (ms)", TextType::Normal))
                        .push(text_input(
                            &mut self.input_sep,
                            "Enter separation time",
                            &self.str_sep,
                            DiagManualMessage::SepEnter,
                        )),
                )
                .push(
                    Column::new()
                        .spacing(2)
                        .width(Length::FillPortion(1)) // Fourth input column
                        .push(text("Block size", TextType::Normal))
                        .push(text_input(
                            &mut self.input_bs,
                            "Enter block size",
                            &self.str_bs,
                            DiagManualMessage::BsEnter,
                        )),
                ),
        );

        let send = Self::decode_string_hex(&self.str_send_id);
        let recv = Self::decode_string_hex(&self.str_recv_id);
        let bs = Self::decode_string_int(&self.str_bs);
        let sep = Self::decode_string_int(&self.str_sep);

        let can_launch = send.is_some() && recv.is_some() && bs.is_some() && sep.is_some();

        let mut kwp_btn_2 = button_outlined(
            &mut self.kwp_btn_state_2,
            "Launch KWP2000 session",
            ButtonType::Primary,
        )
        .width(Length::Units(250));
        if can_launch {
            kwp_btn_2 = kwp_btn_2.on_press(DiagManualMessage::LaunchKWPCustom);
        }

        let mut uds_btn_2 = button_outlined(
            &mut self.uds_btn_state_2,
            "Launch UDS session",
            ButtonType::Primary,
        )
        .width(Length::Units(250));
        if can_launch {
            uds_btn_2 = uds_btn_2.on_press(DiagManualMessage::LaunchUDSCustom);
        }

        let mut cust_btn_2 = button_outlined(
            &mut self.custom_btn_state_2,
            "Launch Custom session",
            ButtonType::Primary,
        )
        .width(Length::Units(250));
        if can_launch {
            cust_btn_2 = cust_btn_2.on_press(DiagManualMessage::LaunchCustomCustom);
        }

        view = view.push(
            Row::new()
                .padding(5)
                .spacing(5)
                .push(kwp_btn_2)
                .push(uds_btn_2)
                .push(cust_btn_2),
        );

        view = view.push(text(&self.status, TextType::Danger));

        Scrollable::new(&mut self.scroll_state)
            .height(Length::Fill)
            .push(view)
            .into()
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::commapi::slcan_api::SlcanApi;

    fn egs52_connection_fixture() -> OvdECU {
        serde_json::from_str(
            r#"{
                "name": "EGS52", "description": "Connection selection fixture", "variants": [],
                "connections": [
                    {
                        "baud": 10400, "send_id": 32, "global_send_id": 32, "recv_id": 32,
                        "server_type": "KWP2000",
                        "connection_type": {"LIN": {
                            "max_segment_size": 254, "wake_up_method": "FiveBaudInit"
                        }}
                    },
                    {
                        "baud": 500000, "send_id": 2017, "recv_id": 2025,
                        "server_type": "KWP2000",
                        "connection_type": {"ISOTP": {
                            "blocksize": 8, "st_min": 10,
                            "ext_can_addr": false, "ext_isotp_addr": false
                        }}
                    }
                ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn multiple_json_connections_require_selection_and_allow_can_retry() {
        let mut manual = DiagManual::new(Box::new(SlcanApi::new(String::new())));
        manual.load_json_ecu(egs52_connection_fixture());
        assert!(manual.status.is_empty());
        assert!(manual.json_connection.is_none());
        assert!(manual.session.is_none());

        manual.update(&DiagManualMessage::LaunchJSON);
        assert_eq!(manual.status, "Select an ECU JSON connection first");
        manual.update(&DiagManualMessage::PickConnection(1));
        assert!(manual.status.is_empty());
        let connection = &manual.json_ecu.as_ref().unwrap().connections[1];
        assert_eq!(connection.baud, 500000);
        assert_eq!(connection.send_id, 0x7e1);
        assert_eq!(connection.recv_id, 0x7e9);

        // No COM port is opened: reaching this error proves the ISO-TP launch path was used.
        manual.update(&DiagManualMessage::LaunchJSON);
        assert!(manual.status.contains("SLCAN serial port is not open"));
        assert!(manual.session.is_none());
        assert_eq!(manual.json_connection, Some(1));
        assert_eq!(manual.json_ecu.as_ref().unwrap().connections.len(), 2);
    }

    #[test]
    fn unsupported_json_connections_are_rejected_before_launch() {
        let mut manual = DiagManual::new(Box::new(SlcanApi::new(String::new())));
        manual.load_json_ecu(egs52_connection_fixture());
        manual.update(&DiagManualMessage::PickConnection(0));
        manual.update(&DiagManualMessage::LaunchJSON);
        assert_eq!(manual.status, "K-Line is not implemented at this time");
        assert!(manual.session.is_none());

        let connection = &manual.json_ecu.as_ref().unwrap().connections[1];
        assert!(DiagManual::json_connection_error(connection, Capability::Yes).is_none());
        assert!(DiagManual::json_connection_error(connection, Capability::No).is_some());
        assert!(DiagManual::json_connection_error(connection, Capability::NA).is_some());
    }

    #[test]
    fn loading_json_resets_selection_and_preserves_single_connection_launch() {
        let mut manual = DiagManual::new(Box::new(SlcanApi::new(String::new())));
        let mut ecu = egs52_connection_fixture();
        ecu.connections.remove(0);
        manual.load_json_ecu(ecu.clone());
        assert_eq!(manual.json_connection, Some(0));
        assert!(manual.status.contains("SLCAN serial port is not open"));

        manual.load_json_ecu(egs52_connection_fixture());
        assert!(manual.json_connection.is_none());
        assert!(manual.status.is_empty());
        manual.update(&DiagManualMessage::PickConnection(1));
        manual.update(&DiagManualMessage::PickConnection(99));
        assert!(manual.json_connection.is_none());

        ecu.connections.clear();
        manual.load_json_ecu(ecu);
        assert!(manual.json_connection.is_none());
        assert_eq!(manual.status, "ECU JSON file has no connections");
        assert!(manual.session.is_none());
    }
}
