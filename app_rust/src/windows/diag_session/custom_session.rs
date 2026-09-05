use iced::Subscription;

use crate::commapi::comm_api::{ComServer, ISO15765Config};

use super::{DiagMessageTrait, SessionError, SessionResult, SessionTrait};

#[derive(Debug, Clone, PartialEq)]
pub enum CustomDiagSessionMsg {
    Back,
}

impl DiagMessageTrait for CustomDiagSessionMsg {
    fn is_back(&self) -> bool {
        self == &CustomDiagSessionMsg::Back
    }
}

#[derive(Debug, Clone)]
pub struct CustomDiagSession;

impl CustomDiagSession {
    pub fn new(_comm_server: Box<dyn ComServer>, _ecu: ISO15765Config) -> SessionResult<Self> {
        Err(SessionError::Other(
            "Custom Session is not yet implemented".into(),
        ))
    }
}

impl SessionTrait for CustomDiagSession {
    type Msg = CustomDiagSessionMsg;

    fn view(&mut self) -> iced::Element<'_, Self::Msg> {
        todo!()
    }

    fn update(&mut self, _msg: &Self::Msg) -> Option<Self::Msg> {
        todo!()
    }

    fn subscription(&self) -> iced::Subscription<Self::Msg> {
        Subscription::none()
    }
}
