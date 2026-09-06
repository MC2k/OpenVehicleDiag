use crate::commapi::{
    iface::{InterfaceConfig, InterfaceType, PayloadFlag, IFACE_CFG},
    protocols::{kwp2000::read_ecu_identification, DiagCfg},
};
use common::schema::{
    diag::{dtc::ECUDTC, service::Service},
    variant::{ECUVariantDefinition, ECUVariantPattern},
    ConType, Connection, OvdECU, ServerType,
};
use core::panic;
use iced::{time, Align, Column, Length, Row, Subscription};
use std::{cell::RefCell, time::Instant, vec};

use crate::{
    commapi::{
        comm_api::ComServer,
        protocols::{DTCState, DiagProtocol, DiagServer, ProtocolResult},
    },
    themes::{
        button_coloured, button_outlined, picklist, text, text_input, title_text, ButtonType,
        TextType,
    },
    widgets::table::{Table, TableMsg},
};

use super::{
    log_view::{LogType, LogView},
    DiagMessageTrait, SessionError, SessionResult, SessionTrait,
};

const TABLE_DTC: usize = 0;
const ENV_TABLE: usize = 1;
const INFO_TABLE_ID: usize = 2;

const MAX_TABLES: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonDiagSessionMsg {
    SelectVariant(VariantChoice),
    ReadErrors,
    ClearErrors,
    ReadInfo,
    SetKwpSession(u8),
    EnterSecurityLevel(String),
    RequestSecuritySeed,
    EnterSecuritySeed(String),
    EnterSecurityKey(String),
    SendSecurityKey,
    ExecuteService(ServiceRef, Vec<u8>),
    ClearLogs,
    Selector(SelectorMsg),
    LoopRead(Instant),
    Navigate(TargetPage),
    Select(usize, usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantChoice {
    index: usize,
    label: String,
}

impl ToString for VariantChoice {
    fn to_string(&self) -> String {
        self.label.clone()
    }
}

impl From<SelectorMsg> for JsonDiagSessionMsg {
    fn from(x: SelectorMsg) -> Self {
        JsonDiagSessionMsg::Selector(x)
    }
}

impl DiagMessageTrait for JsonDiagSessionMsg {
    fn is_back(&self) -> bool {
        match self {
            JsonDiagSessionMsg::Navigate(s) => s == &TargetPage::Home,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DisplayableDTC {
    code: String, // DTC Itself
    desc: String,
    state: DTCState,
    mil_on: bool,
    envs: Vec<(String, String)>, // Key, Value
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum TargetPage {
    Home,
    Main,
    ECUInfo,
    Error,
}

#[derive(Debug, Clone)]
pub struct JsonDiagSession {
    connection_settings: Connection,
    server: DiagServer,
    available_variants: Vec<ECUVariantDefinition>,
    variant_options: Vec<VariantChoice>,
    selected_variant: VariantChoice,
    variant_picker: iced::pick_list::State<VariantChoice>,
    ecu_text: (String, String),
    ecu_data: ECUVariantDefinition,
    pattern: ECUVariantPattern,
    log_view: LogView,
    service_selector: ServiceSelector,
    looping_text: String,
    looping_service: Option<ServiceRef>, // Allow only read-only services to be loop read
    logged_dtcs: Vec<DisplayableDTC>,    // DTCs stored on ECU,
    btn1: iced::button::State,
    btn2: iced::button::State,
    btn3: iced::button::State,
    kwp_default_btn: iced::button::State,
    kwp_standby_btn: iced::button::State,
    kwp_passive_btn: iced::button::State,
    kwp_extended_btn: iced::button::State,
    security_level: String,
    security_seed: String,
    security_key: String,
    security_level_input: iced::text_input::State,
    security_seed_input: iced::text_input::State,
    security_key_input: iced::text_input::State,
    security_seed_btn: iced::button::State,
    security_key_btn: iced::button::State,
    page_state: TargetPage,
    tables: Vec<Table>,
}

fn service_selector_for_variant(variant: &ECUVariantDefinition) -> ServiceSelector {
    let read_functions = variant
        .downloads
        .iter()
        .map(|service| ServiceRef {
            inner: RefCell::new(service.clone()),
        })
        .collect();
    let write_functions = variant
        .functions
        .iter()
        .map(|service| ServiceRef {
            inner: RefCell::new(service.clone()),
        })
        .collect();

    ServiceSelector::new(read_functions, write_functions, Vec::new())
}

impl JsonDiagSession {
    pub fn new(
        comm_server: Box<dyn ComServer>,
        ecu_data: OvdECU,
        connection_settings: Connection,
    ) -> SessionResult<Self> {
        let diag_server_type = match connection_settings.server_type {
            common::schema::ServerType::UDS => DiagProtocol::UDS,
            common::schema::ServerType::KWP2000 => DiagProtocol::KWP2000,
        };
        println!("Detect. ECU uses {:?}", diag_server_type);

        // TODO K-Line KWP2000
        // For now, Diag server ONLY supports ISO-TP, not LIN!
        let create_server = match connection_settings.connection_type {
            ConType::ISOTP {
                blocksize,
                st_min,
                ext_isotp_addr,
                ext_can_addr,
            } => {
                let mut cfg = InterfaceConfig::new();
                cfg.add_param(IFACE_CFG::BAUDRATE, connection_settings.baud);
                cfg.add_param(IFACE_CFG::EXT_CAN_ADDR, ext_can_addr as u32);
                cfg.add_param(IFACE_CFG::EXT_ISOTP_ADDR, ext_isotp_addr as u32);
                cfg.add_param(IFACE_CFG::ISOTP_BS, blocksize);
                cfg.add_param(IFACE_CFG::ISOTP_ST_MIN, st_min);

                let diag_cfg = DiagCfg {
                    send_id: connection_settings.send_id,
                    recv_id: connection_settings.recv_id,
                    global_id: connection_settings.global_send_id,
                };

                let tx_flags = vec![PayloadFlag::ISOTP_PAD_FRAME];
                DiagServer::new(
                    diag_server_type,
                    &comm_server,
                    InterfaceType::IsoTp,
                    cfg,
                    Some(tx_flags),
                    diag_cfg,
                )
            }
            ConType::LIN { .. } => {
                return Err(SessionError::Other(
                    "K-Line is not implemented at this time".into(),
                ))
            }
        };

        match create_server {
            Ok(server) => {
                println!("Server started");
                // Some ECUs reject the optional KWP2000 identification service (0x1A).
                // Keep the session usable and fall back to the first CBF variant.
                let variant = match server.get_variant_id() {
                    Ok(variant) => Some(variant as u32),
                    Err(error) => {
                        eprintln!("WARNING. ECU variant identification failed: {}", error.get_text());
                        None
                    }
                };
                let selected_variant_index = ecu_data
                    .variants
                    .iter()
                    .position(|x| {
                        variant.map_or(false, |variant| {
                            x.patterns.iter().any(|p| p.vendor_id == variant)
                        })
                    })
                    .unwrap_or_else(|| {
                        eprintln!("WARNING. Unknown ECU Variant!");
                        0
                    });
                let ecu_varient = ecu_data
                    .variants
                    .get(selected_variant_index)
                    .cloned()
                    .ok_or_else(|| SessionError::Other("ECU has no variants".into()))?;
                let pattern = ecu_varient
                    .patterns
                    .iter()
                    .find(|x| variant.map_or(false, |variant| x.vendor_id == variant))
                    .or_else(|| ecu_varient.patterns.first())
                    .ok_or_else(|| SessionError::Other("ECU variant has no identification patterns".into()))?
                    .clone();
                let available_variants = ecu_data.variants.clone();
                let variant_options: Vec<VariantChoice> = available_variants
                    .iter()
                    .enumerate()
                    .map(|(index, variant)| VariantChoice {
                        index,
                        label: format!("{} - {}", variant.name, variant.description),
                    })
                    .collect();
                let selected_variant = variant_options[selected_variant_index].clone();
                println!(
                    "ECU Variant: {} (Vendor: {})",
                    ecu_varient.name, pattern.vendor
                );

                let read_functions: Vec<ServiceRef> = ecu_varient
                    .downloads
                    .iter()
                    .clone()
                    .map(|s| ServiceRef {
                        inner: RefCell::new(s.clone()),
                    })
                    .collect();

                let write_functions: Vec<ServiceRef> = ecu_varient
                    .functions
                    .iter()
                    .map(|s| ServiceRef {
                        inner: RefCell::new(s.clone()),
                    })
                    .collect();
                let actuation_functions: Vec<ServiceRef> = Vec::new();

                Ok(Self {
                    connection_settings: connection_settings,
                    ecu_text: (ecu_data.name, ecu_data.description),
                    server,
                    available_variants,
                    variant_options,
                    selected_variant,
                    variant_picker: Default::default(),
                    ecu_data: ecu_varient,
                    pattern: pattern.clone(),
                    service_selector: ServiceSelector::new(
                        read_functions,
                        write_functions,
                        actuation_functions,
                    ),
                    log_view: LogView::new(),
                    btn1: iced::button::State::default(),
                    btn2: iced::button::State::default(),
                    btn3: iced::button::State::default(),
                    kwp_default_btn: iced::button::State::default(),
                    kwp_standby_btn: iced::button::State::default(),
                    kwp_passive_btn: iced::button::State::default(),
                    kwp_extended_btn: iced::button::State::default(),
                    security_level: "01".into(),
                    security_seed: String::new(),
                    security_key: String::new(),
                    security_level_input: iced::text_input::State::default(),
                    security_seed_input: iced::text_input::State::default(),
                    security_key_input: iced::text_input::State::default(),
                    security_seed_btn: iced::button::State::default(),
                    security_key_btn: iced::button::State::default(),
                    looping_service: None,
                    looping_text: String::new(),
                    logged_dtcs: Vec::new(),
                    page_state: TargetPage::Main,
                    tables: vec![Table::default(); MAX_TABLES],
                })
            }
            Err(e) => {
                eprintln!("Could not setup diag server");
                Err(SessionError::ServerError(e))
            }
        }
    }
}

impl JsonDiagSession {
    fn security_level(value: &str) -> Result<u8, &'static str> {
        match hex::decode(value) {
            Ok(bytes) if bytes.len() == 1 => Ok(bytes[0]),
            _ => Err("Security level must be exactly one hexadecimal byte, for example 01"),
        }
    }

    pub fn draw_main_ui(&mut self) -> iced::Element<'_, JsonDiagSessionMsg> {
        let mut btn_view = Column::new()
            .push(text("Security Access (0x27)", TextType::Normal))
            .push(
                Row::new()
                    .spacing(5)
                    .push(
                        text_input(
                            &mut self.security_level_input,
                            "Seed level",
                            &self.security_level,
                            JsonDiagSessionMsg::EnterSecurityLevel,
                        )
                        .width(Length::Units(90)),
                    )
                    .push(
                        button_outlined(
                            &mut self.security_seed_btn,
                            "Request seed",
                            ButtonType::Warning,
                        )
                        .on_press(JsonDiagSessionMsg::RequestSecuritySeed),
                    )
                    .push(
                        text_input(
                            &mut self.security_seed_input,
                            "Returned seed",
                            &self.security_seed,
                            JsonDiagSessionMsg::EnterSecuritySeed,
                        )
                        .width(Length::Units(180)),
                    )
                    .push(
                        text_input(
                            &mut self.security_key_input,
                            "Calculated key",
                            &self.security_key,
                            JsonDiagSessionMsg::EnterSecurityKey,
                        )
                        .width(Length::Units(180)),
                    )
                    .push(
                        button_outlined(&mut self.security_key_btn, "Send key", ButtonType::Danger)
                            .on_press(JsonDiagSessionMsg::SendSecurityKey),
                    ),
            )
            .push(
                self.service_selector
                    .view()
                    .map(JsonDiagSessionMsg::Selector),
            )
            .push(
                button_outlined(&mut self.btn1, "ECU Information", ButtonType::Primary)
                    .on_press(JsonDiagSessionMsg::ReadInfo),
            )
            .width(Length::FillPortion(1))
            .push(
                button_outlined(&mut self.btn2, "Read errors", ButtonType::Primary)
                    .on_press(JsonDiagSessionMsg::ReadErrors),
            )
            .width(Length::FillPortion(1));
        if matches!(&self.connection_settings.server_type, ServerType::KWP2000) {
            btn_view = btn_view.push(
                Row::new()
                    .spacing(5)
                    .push(text("KWP session:", TextType::Normal))
                    .push(
                        button_outlined(&mut self.kwp_default_btn, "Default 81", ButtonType::Info)
                            .on_press(JsonDiagSessionMsg::SetKwpSession(0x81)),
                    )
                    .push(
                        button_outlined(&mut self.kwp_standby_btn, "Standby 89", ButtonType::Info)
                            .on_press(JsonDiagSessionMsg::SetKwpSession(0x89)),
                    )
                    .push(
                        button_outlined(&mut self.kwp_passive_btn, "Passive 90", ButtonType::Info)
                            .on_press(JsonDiagSessionMsg::SetKwpSession(0x90)),
                    )
                    .push(
                        button_outlined(
                            &mut self.kwp_extended_btn,
                            "Extended 92",
                            ButtonType::Info,
                        )
                        .on_press(JsonDiagSessionMsg::SetKwpSession(0x92)),
                    ),
            );
        }
        if self.variant_options.len() > 1 {
            btn_view = btn_view.push(
                Row::new()
                    .spacing(5)
                    .push(text("ECU variant:", TextType::Normal))
                    .push(picklist(
                        &mut self.variant_picker,
                        &self.variant_options,
                        Some(self.selected_variant.clone()),
                        JsonDiagSessionMsg::SelectVariant,
                    )),
            );
        }
        if self.looping_service.is_some() {
            btn_view = btn_view.push(text(&self.looping_text, TextType::Normal).size(14));
        }
        Column::new()
            .align_items(Align::Center)
            .spacing(8)
            .padding(8)
            .push(title_text(
                format!(
                    "ECU: {} ({}). DiagVersion: {}, Vendor: {}",
                    self.ecu_text.0, self.ecu_text.1, self.ecu_data.name, self.pattern.vendor
                )
                .as_str(),
                crate::themes::TitleSize::P4,
            ))
            .push(text(
                format!("Automatic connection method!: Using {:?} at {}bps with {:?} a diagnostic server", 
                    &self.connection_settings.connection_type,
                    &self.connection_settings.baud,
                    &self.connection_settings.server_type,
                ).as_str(),
                TextType::Disabled,
            ))
            .push(
                Row::new().spacing(8).padding(8).push(btn_view).push(
                    Column::new()
                        .push(self.log_view.view(JsonDiagSessionMsg::ClearLogs))
                        .width(Length::FillPortion(1)),
                ),
            ).into()
    }

    pub fn draw_error_ui(&mut self) -> iced::Element<'_, JsonDiagSessionMsg> {
        // Create a table of errors
        // Top row (Clear + back button)

        let mut content = Column::new()
            .padding(8)
            .spacing(8)
            .align_items(Align::Center);

        let header = Row::new()
            .padding(8)
            .spacing(8)
            .align_items(Align::Center)
            .push(title_text("ECU Error view", crate::themes::TitleSize::P3));

        // Clear btn
        let read_btn = button_outlined(&mut self.btn1, "Read errors", ButtonType::Primary)
            .on_press(JsonDiagSessionMsg::ReadErrors);

        let mut clear_btn = button_outlined(&mut self.btn2, "Clear errors", ButtonType::Primary);
        if self.logged_dtcs.len() > 0 {
            clear_btn = clear_btn.on_press(JsonDiagSessionMsg::ClearErrors);
        };

        let btn_row = Row::new()
            .padding(8)
            .spacing(8)
            .push(read_btn)
            .push(clear_btn)
            .push(iced::Space::with_width(Length::Fill))
            .push(
                button_outlined(&mut self.btn3, "Back", ButtonType::Primary)
                    .on_press(JsonDiagSessionMsg::Navigate(TargetPage::Main)),
            );

        // Like Vediamo. 2 tables. 1 with error list, 1 with env data for current selected DTC

        content = content.push(header);
        content = content.push(btn_row);

        if self.logged_dtcs.len() > 0 {
            content = content.align_items(Align::Start);

            for (id, table) in self.tables.iter_mut().enumerate() {
                match id {
                    TABLE_DTC => {
                        content = content.push(
                            table
                                .view()
                                .map(|x| JsonDiagSessionMsg::Select(TABLE_DTC, x.0, x.1)),
                        );
                    }
                    ENV_TABLE => {
                        content = content.push(title_text(
                            "Freeze frame data",
                            crate::themes::TitleSize::P4,
                        ));
                        content = content.push(
                            table
                                .view()
                                .map(|x| JsonDiagSessionMsg::Select(ENV_TABLE, x.0, x.1)),
                        );
                    }
                    _ => {}
                }
            }
            content.into()
        } else {
            //text("No Diagnostic trouble codes found", TextType::Normal)
            content.push(text("No DTCs", TextType::Normal)).into()
        }
    }

    pub fn draw_info_ui(&mut self) -> iced::Element<'_, JsonDiagSessionMsg> {
        let content = Column::new()
            .padding(8)
            .spacing(8)
            .align_items(Align::Center);

        let header = Row::new()
            .padding(8)
            .spacing(8)
            .align_items(Align::Center)
            .push(title_text("ECU Info view", crate::themes::TitleSize::P3));

        let btn_row = Row::new()
            .padding(8)
            .spacing(8)
            .push(iced::Space::with_width(Length::Fill))
            .push(
                button_outlined(&mut self.btn1, "Back", ButtonType::Primary)
                    .on_press(JsonDiagSessionMsg::Navigate(TargetPage::Main)),
            );

        content
            .push(header)
            .push(btn_row)
            .push(
                self.tables[INFO_TABLE_ID]
                    .view()
                    .map(|_| JsonDiagSessionMsg::Select(INFO_TABLE_ID, 0, 0)),
            )
            .into()
    }
}

impl SessionTrait for JsonDiagSession {
    type Msg = JsonDiagSessionMsg;
    fn view(&mut self) -> iced::Element<'_, Self::Msg> {
        match self.page_state {
            TargetPage::Main => self.draw_main_ui(),
            TargetPage::Error => self.draw_error_ui(),
            TargetPage::ECUInfo => self.draw_info_ui(),
            _ => panic!("????"),
        }
        .into()
    }

    fn update(&mut self, msg: &Self::Msg) -> Option<Self::Msg> {
        //self.log_view.clear_logs();
        match msg {
            JsonDiagSessionMsg::SelectVariant(choice) => {
                if let Some(variant) = self.available_variants.get(choice.index).cloned() {
                    if let Some(pattern) = variant.patterns.first().cloned() {
                        self.ecu_data = variant.clone();
                        self.pattern = pattern;
                        self.selected_variant = choice.clone();
                        self.service_selector = service_selector_for_variant(&variant);
                        self.looping_service = None;
                    } else {
                        self.log_view.add_msg(
                            "Selected ECU variant has no identification pattern",
                            LogType::Error,
                        );
                    }
                }
            }
            JsonDiagSessionMsg::Navigate(target) => self.page_state = *target,
            JsonDiagSessionMsg::ReadInfo => {
                let header: Vec<String> = vec!["".into(), "".into()];
                let mut params: Vec<Vec<String>> = Vec::new();
                params.push(vec!["Name".into(), self.ecu_text.0.clone()]);
                params.push(vec!["Description".into(), self.ecu_text.1.clone()]);
                params.push(vec!["Software".into(), self.ecu_data.name.clone()]);
                params.push(vec!["Manufacture".into(), self.pattern.vendor.clone()]);
                if let Some(kwp) = self.server.into_kwp() {
                    if let Ok(res) = read_ecu_identification::read_dcx_mmc_id(kwp) {
                        params.push(vec!["Part number".into(), res.part_number.clone()]);
                        params.push(vec![
                            "Hardware version".into(),
                            res.hardware_version.clone(),
                        ]);
                        params.push(vec![
                            "Software version".into(),
                            res.software_version.clone(),
                        ]);
                    } else {
                        params.push(vec!["Part number".into(), "Unknown".into()]);
                        params.push(vec!["Hardware version".into(), "Unknown".into()]);
                        params.push(vec!["Software version".into(), "Unknown".into()]);
                    }

                    if let Ok(res) = read_ecu_identification::read_dcs_id(kwp) {
                        params.push(vec![
                            "Hardware build date (WW/YY)".into(),
                            res.hardware_build_date.clone(),
                        ]);
                        params.push(vec![
                            "Software build date (WW/YY)".into(),
                            res.software_written_date.clone(),
                        ]);
                        params.push(vec![
                            "Production date (DD/MM/YY)".into(),
                            res.production_date.clone(),
                        ]);
                    } else {
                        params.push(vec!["Hardware build date (WW/YY)".into(), "Unknown".into()]);
                        params.push(vec!["Software build date (WW/YY)".into(), "Unknown".into()]);
                        params.push(vec!["Production date (DD/MM/YY)".into(), "Unknown".into()]);
                    }
                }

                self.tables[INFO_TABLE_ID] = Table::new(header, params, vec![400, 400], false, 900);
                return Some(JsonDiagSessionMsg::Navigate(TargetPage::ECUInfo));
            }
            JsonDiagSessionMsg::ReadErrors => match self.server.read_errors() {
                Ok(res) => {
                    let dtc_list = self.ecu_data.errors.clone();
                    self.logged_dtcs = res
                        .iter()
                        .map(|raw_dtc| {
                            let ecu_dtc = dtc_list
                                .clone()
                                .into_iter()
                                .find(|x| x.error_name.ends_with(&raw_dtc.error))
                                .unwrap_or(ECUDTC {
                                    error_name: raw_dtc.error.clone(),
                                    summary: "UNKNOWN ERROR".into(),
                                    description: "UNKNOWN DTC".into(),
                                    envs: Vec::new(),
                                });
                            let mut res = DisplayableDTC {
                                code: ecu_dtc.error_name.clone(),
                                desc: ecu_dtc.description.clone(),
                                state: raw_dtc.state,
                                mil_on: raw_dtc.check_engine_on,
                                envs: Vec::new(),
                            };

                            if ecu_dtc.envs.len() > 0 {
                                // If parsable freeze frame data exists, then read it
                                if let Ok(args) = self.server.get_dtc_env_data(raw_dtc) {
                                    for e in &ecu_dtc.envs {
                                        match e.decode_value_to_string(&args) {
                                            Ok(s) => res.envs.push((e.name.clone(), s)),
                                            Err(err) => {
                                                eprintln!(
                                                    "Warning could not decode param: {:?}",
                                                    err
                                                )
                                            }
                                        }
                                    }
                                }
                            }
                            res
                        })
                        .collect();
                    let entries: Vec<Vec<String>> = self
                        .logged_dtcs
                        .iter()
                        .map(|dtc| {
                            vec![
                                dtc.code.clone(),
                                dtc.desc.clone(),
                                format!("{:?}", dtc.state),
                                if dtc.mil_on {
                                    "YES".into()
                                } else {
                                    "NO ".into()
                                },
                            ]
                        })
                        .collect();

                    let table = Table::new(
                        vec![
                            "Error".into(),
                            "Description".into(),
                            "State".into(),
                            "MIL on".into(),
                        ],
                        entries,
                        vec![200, 600, 150, 100],
                        true,
                        400,
                    );
                    self.tables[TABLE_DTC] = table;
                    if self.page_state == TargetPage::Main {
                        // Goto DTC View!
                        return Some(JsonDiagSessionMsg::Navigate(TargetPage::Error));
                    }
                }
                Err(e) => {
                    self.log_view.add_msg(
                        format!("Error reading ECU Errors: {}", e.get_text()),
                        LogType::Error,
                    );
                    self.logged_dtcs.clear();
                }
            },
            JsonDiagSessionMsg::ClearErrors => match self.server.clear_errors() {
                Ok(_) => {
                    self.log_view.add_msg("Clear ECU Errors OK!", LogType::Info);
                    self.logged_dtcs.clear();
                }
                Err(e) => self.log_view.add_msg(
                    format!("Error clearing ECU Errors: {}", e.get_text()),
                    LogType::Error,
                ),
            },

            JsonDiagSessionMsg::SetKwpSession(mode) => match self.server.set_kwp_session(*mode) {
                Ok(_) => self.log_view.add_msg(
                    format!("KWP diagnostic session set to 0x{:02X}", mode),
                    LogType::Info,
                ),
                Err(e) => self.log_view.add_msg(
                    format!("Error setting KWP diagnostic session: {}", e.get_text()),
                    LogType::Error,
                ),
            },

            JsonDiagSessionMsg::EnterSecurityLevel(level) => {
                self.security_level = level.to_uppercase();
            }
            JsonDiagSessionMsg::EnterSecuritySeed(seed) => {
                self.security_seed = seed.to_uppercase();
            }
            JsonDiagSessionMsg::EnterSecurityKey(key) => {
                self.security_key = key.to_uppercase();
            }
            JsonDiagSessionMsg::RequestSecuritySeed => {
                let level = match Self::security_level(&self.security_level) {
                    Ok(level) => level,
                    Err(error) => {
                        self.log_view.add_msg(error, LogType::Error);
                        return None;
                    }
                };
                match self.server.run_cmd(0x27, &[level]) {
                    Ok(response)
                        if response.len() >= 2 && response[0] == 0x67 && response[1] == level =>
                    {
                        self.security_seed = hex::encode_upper(&response[2..]);
                        self.log_view.add_msg(
                            format!(
                                "Security seed for level {:02X}: {}",
                                level, self.security_seed
                            ),
                            LogType::Info,
                        );
                    }
                    Ok(response) => self.log_view.add_msg(
                        format!(
                            "Unexpected Security Access seed response: {:02X?}",
                            response
                        ),
                        LogType::Error,
                    ),
                    Err(error) => self.log_view.add_msg(
                        format!(
                            "Error requesting Security Access seed: {}",
                            error.get_text()
                        ),
                        LogType::Error,
                    ),
                }
            }
            JsonDiagSessionMsg::SendSecurityKey => {
                let seed_level = match Self::security_level(&self.security_level) {
                    Ok(level) => level,
                    Err(error) => {
                        self.log_view.add_msg(error, LogType::Error);
                        return None;
                    }
                };
                let key_level = match seed_level.checked_add(1) {
                    Some(level) => level,
                    None => {
                        self.log_view
                            .add_msg("Security level must be below FF", LogType::Error);
                        return None;
                    }
                };
                let key = match hex::decode(&self.security_key) {
                    Ok(key) if !key.is_empty() => key,
                    _ => {
                        self.log_view
                            .add_msg("Enter a non-empty hexadecimal security key", LogType::Error);
                        return None;
                    }
                };
                let mut request = vec![key_level];
                request.extend(key);
                match self.server.run_cmd(0x27, &request) {
                    Ok(response)
                        if response.len() >= 2
                            && response[0] == 0x67
                            && response[1] == key_level =>
                    {
                        self.log_view
                            .add_msg("Security Access unlocked", LogType::Info);
                    }
                    Ok(response) => self.log_view.add_msg(
                        format!("Unexpected Security Access key response: {:02X?}", response),
                        LogType::Error,
                    ),
                    Err(error) => self.log_view.add_msg(
                        format!("Error sending Security Access key: {}", error.get_text()),
                        LogType::Error,
                    ),
                }
            }

            JsonDiagSessionMsg::Selector(s) => match s {
                SelectorMsg::PickLoopService(l) => self.looping_service = Some(l.clone()),
                SelectorMsg::StopLoopService => {
                    self.looping_service = None;
                    return self.service_selector.update(s);
                }
                _ => return self.service_selector.update(s),
            },

            JsonDiagSessionMsg::ExecuteService(s, args) => {
                println!("Exec {}", s.inner.borrow().name);
                match s.exec(args, &mut self.server) {
                    Ok(res) => self.log_view.add_log(
                        format!(
                            "{} ({}):",
                            s.inner.borrow().name,
                            s.inner.borrow().description
                        ),
                        s.args_to_string(&res),
                        LogType::Info,
                    ),
                    Err(e) => self.log_view.add_msg(
                        format!("Error executing {}: {:?}", s.inner.borrow().name, e).as_str(),
                        LogType::Error,
                    ),
                }
            }
            JsonDiagSessionMsg::ClearLogs => self.log_view.clear_logs(),
            JsonDiagSessionMsg::LoopRead(_) => {
                if let Some(s) = &self.looping_service {
                    if let Ok(res) = s.exec(&[], &mut self.server) {
                        self.looping_text = format!(
                            "{}({})\n->{}",
                            s.inner.borrow().name,
                            s.inner.borrow().description,
                            s.args_to_string(&res)
                        )
                    }
                }
            }
            JsonDiagSessionMsg::Select(table_id, x, y) => {
                // update the table!
                self.tables[*table_id].update(&TableMsg(*x, *y));
                if *table_id == TABLE_DTC {
                    let header = vec!["Parameter".into(), "Value".into()];
                    let mut values: Vec<Vec<String>> = Vec::new();
                    for (name, v) in &self.logged_dtcs[*y].envs {
                        values.push(vec![name.clone(), v.clone()]);
                    }
                    self.tables[ENV_TABLE] = Table::new(header, values, vec![400, 200], false, 300);
                }
            }
        }
        None
    }

    fn subscription(&self) -> iced::Subscription<Self::Msg> {
        if self.looping_service.is_some() {
            return time::every(std::time::Duration::from_millis(500))
                .map(JsonDiagSessionMsg::LoopRead);
        }
        Subscription::none()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceRef {
    // Read data from ECU
    inner: RefCell<Service>,
}

impl Eq for ServiceRef {}

impl ServiceRef {
    pub fn match_query(&self, q: &str) -> bool {
        return self.inner.borrow().name.to_lowercase().contains(q);
    }

    pub fn require_input(&self) -> bool {
        return !self.inner.borrow().input_params.is_empty();
    }

    pub fn exec(&self, replace_args: &[u8], server: &mut DiagServer) -> ProtocolResult<Vec<u8>> {
        let p = &self.inner.borrow().payload;
        let mut args = if p.is_empty() {
            Vec::new()
        } else {
            Vec::from(&p[1..])
        };
        if !replace_args.is_empty() && replace_args.len() <= args.len() {
            for (pos, x) in replace_args.iter().enumerate() {
                args[pos] |= x;
            }
        }
        server.run_cmd(self.inner.borrow().payload[0], &args)
    }

    pub fn args_to_string(&self, args: &[u8]) -> String {
        let outputs = &self.inner.borrow().output_params;
        if outputs.is_empty() {
            "OK".into()
        } else {
            let mut res: String = String::new();
            for o in outputs {
                match o.decode_value_to_string(args) {
                    Ok(r) => res.push_str(format!("{}: {}\n", o.name, r).as_str()),
                    Err(e) => {
                        res.push_str(format!("Error decoding {}: {:?}\n", o.name, e).as_str())
                    }
                }
            }
            res.remove(res.len() - 1);
            res
        }
    }
}

impl ToString for ServiceRef {
    fn to_string(&self) -> String {
        self.inner.borrow().name.clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectorMsg {
    ViewRead,
    ViewWrite,
    ViewActuation,
    PickService(ServiceRef),
    PickLoopService(ServiceRef),
    StopLoopService,
    BeginLoopService,
    ExecService,
    Search(String),
}

#[derive(Debug, Clone)]
pub struct ServiceSelector {
    read_services: Vec<ServiceRef>,
    write_services: Vec<ServiceRef>,
    actuation_services: Vec<ServiceRef>,

    shown_services: Vec<ServiceRef>,

    query_string: String,

    r_btn: iced::button::State,
    w_btn: iced::button::State,
    a_btn: iced::button::State,
    execb: iced::button::State,
    l_btn: iced::button::State,
    is_loop: bool,
    args: Vec<u8>,

    s_bar: iced::text_input::State,

    input_require: bool,
    can_execute: bool,

    selected_service: Option<ServiceRef>,
    picker: iced::pick_list::State<ServiceRef>,

    view_selection: [bool; 3], // Read, Write, Actuation
}

impl ServiceSelector {
    pub fn new(r: Vec<ServiceRef>, w: Vec<ServiceRef>, a: Vec<ServiceRef>) -> Self {
        println!(
            "{} Read services, {} Write services, {} Actuation services",
            r.len(),
            w.len(),
            a.len()
        );

        Self {
            read_services: r.clone(),
            write_services: w,
            actuation_services: a,
            query_string: String::new(),
            r_btn: Default::default(),
            w_btn: Default::default(),
            a_btn: Default::default(),
            s_bar: Default::default(),
            picker: Default::default(),
            execb: Default::default(),
            l_btn: Default::default(),
            args: Vec::new(),
            selected_service: None,
            view_selection: [true, false, false], // Read is default view
            shown_services: r,
            can_execute: false,
            input_require: false,
            is_loop: false,
        }
    }

    pub fn view(&mut self) -> iced::Element<'_, SelectorMsg> {
        let r_btn = match self.view_selection[0] {
            false => button_outlined(&mut self.r_btn, "Read", ButtonType::Info),
            true => button_coloured(&mut self.r_btn, "Read", ButtonType::Info),
        };

        let w_btn = match self.view_selection[1] {
            false => button_outlined(&mut self.w_btn, "Write", ButtonType::Info),
            true => button_coloured(&mut self.w_btn, "Write", ButtonType::Info),
        };

        let a_btn = match self.view_selection[2] {
            false => button_outlined(&mut self.a_btn, "Actuate", ButtonType::Info),
            true => button_coloured(&mut self.a_btn, "Actuate", ButtonType::Info),
        };

        let search_bar = text_input(
            &mut self.s_bar,
            "Search for function",
            self.query_string.as_str(),
            SelectorMsg::Search,
        );

        let btn_row = Row::new()
            .spacing(5)
            .width(Length::Fill)
            .padding(5)
            .push(
                r_btn
                    .on_press(SelectorMsg::ViewRead)
                    .width(Length::FillPortion(1)),
            )
            .push(
                w_btn
                    .on_press(SelectorMsg::ViewWrite)
                    .width(Length::FillPortion(1)),
            )
            .push(
                a_btn
                    .on_press(SelectorMsg::ViewActuation)
                    .width(Length::FillPortion(1)),
            );

        let mut content_view = if self.shown_services.is_empty() {
            Column::new()
                .push(text("No functions match your query", TextType::Normal))
                .spacing(5)
                .width(Length::Fill)
                .padding(5)
        } else {
            Column::new()
                .spacing(5)
                .width(Length::Fill)
                .padding(5)
                .push(text(
                    format!("{} function(s) match your query", self.shown_services.len()).as_str(),
                    TextType::Normal,
                ))
                .push(picklist(
                    &mut self.picker,
                    &self.shown_services,
                    self.selected_service.clone(),
                    SelectorMsg::PickService,
                ))
        };

        if let Some(curr_service) = &self.selected_service {
            content_view = content_view.push(text(
                format!("Description: {}", curr_service.inner.borrow().description).as_str(),
                TextType::Normal,
            ));

            if self.input_require {
                for x in &curr_service.inner.borrow().input_params {
                    content_view = content_view.push(text(
                        format!("Input {} Required. Type: {:?}", x.name, x.data_format).as_str(),
                        TextType::Normal,
                    ))
                }
            }

            if !self.is_loop {
                if self.can_execute {
                    let text = if self.view_selection[0] {
                        "Read "
                    } else if self.view_selection[1] {
                        "Write "
                    } else {
                        "Actuate "
                    };
                    content_view = content_view.push(
                        button_coloured(
                            &mut self.execb,
                            format!("{}{}", text, curr_service.inner.borrow().name).as_str(),
                            ButtonType::Danger,
                        )
                        .on_press(SelectorMsg::ExecService),
                    )
                }
                if self.can_execute == self.view_selection[0] {
                    // Show the graph button
                    content_view = content_view.push(
                        button_coloured(&mut self.l_btn, "Begin graphing", ButtonType::Info)
                            .on_press(SelectorMsg::BeginLoopService),
                    )
                }
            } else {
                // Stop the loop
                content_view = content_view.push(
                    button_coloured(&mut self.l_btn, "Stop graphing", ButtonType::Info)
                        .on_press(SelectorMsg::StopLoopService),
                )
            }
        }

        Column::new()
            .width(Length::Fill)
            .spacing(5)
            .width(Length::Fill)
            .padding(5)
            .push(search_bar.width(Length::Fill))
            .push(btn_row)
            .push(content_view)
            .into()
    }

    pub fn get_shown_services(&self, src: &[ServiceRef]) -> Vec<ServiceRef> {
        if self.query_string.is_empty() {
            return Vec::from(src);
        }
        let lc = self.query_string.to_lowercase();
        src.iter()
            .filter(|x| x.match_query(lc.as_str()))
            .cloned()
            .collect()
    }

    pub fn on_change_items(&mut self) {
        self.selected_service = None;
        self.can_execute = false;
        self.input_require = false;
    }

    pub fn update(&mut self, msg: &SelectorMsg) -> Option<JsonDiagSessionMsg> {
        match &msg {
            SelectorMsg::ViewActuation => {
                self.view_selection = [false, false, true];
                self.shown_services = self.get_shown_services(&self.actuation_services);
                self.on_change_items();
            }
            SelectorMsg::ViewRead => {
                self.view_selection = [true, false, false];
                self.shown_services = self.get_shown_services(&self.read_services);
                self.on_change_items();
            }
            SelectorMsg::ViewWrite => {
                self.view_selection = [false, true, false];
                self.shown_services = self.get_shown_services(&self.write_services);
                self.on_change_items();
            }
            SelectorMsg::Search(s) => {
                let old_len = self.query_string.len();
                self.query_string = s.clone();
                self.shown_services = if old_len < self.query_string.len() {
                    // Adding to existing input
                    self.get_shown_services(&self.shown_services) // Reduce the current array (faster)
                } else {
                    // Reduce the source arrays
                    if self.view_selection[0] {
                        // Read
                        self.get_shown_services(&self.read_services)
                    } else if self.view_selection[1] {
                        // Write
                        self.get_shown_services(&self.write_services)
                    } else {
                        // Actuations
                        self.get_shown_services(&self.actuation_services)
                    }
                }
            }
            SelectorMsg::PickService(s) => {
                if s.require_input() {
                    self.can_execute = false;
                    self.input_require = true;
                } else {
                    self.can_execute = true;
                    self.input_require = false;
                }
                self.selected_service = Some(s.clone());
                println!("{} selected", s.inner.borrow().name);
            }
            SelectorMsg::StopLoopService => {
                self.is_loop = false;
            }
            SelectorMsg::BeginLoopService => {
                if let Some(s) = &self.selected_service {
                    self.is_loop = true;
                    return Some(JsonDiagSessionMsg::Selector(SelectorMsg::PickLoopService(
                        s.clone(),
                    )));
                }
            }
            SelectorMsg::ExecService => {
                return Some(JsonDiagSessionMsg::ExecuteService(
                    self.selected_service.clone().unwrap(),
                    self.args.clone(),
                ))
            }
            _ => {}
        }
        None
    }
}
