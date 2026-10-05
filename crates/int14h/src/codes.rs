use crate::Int14hError;

/// Result of Poll and Service (int14h-api.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkStatus {
    /// Also returned while not connected.
    Ok,
    CarrierLost,
    /// The outstanding frame drew its tenth NAK.
    LineTooNoisy,
    /// The outstanding frame timed out for the twelfth time.
    NotAcknowledging,
}

impl LinkStatus {
    pub const fn code(self) -> u8 {
        match self {
            LinkStatus::Ok => 0,
            LinkStatus::CarrierLost => 1,
            LinkStatus::LineTooNoisy => 2,
            LinkStatus::NotAcknowledging => 3,
        }
    }
}

impl TryFrom<u8> for LinkStatus {
    type Error = Int14hError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(LinkStatus::Ok),
            1 => Ok(LinkStatus::CarrierLost),
            2 => Ok(LinkStatus::LineTooNoisy),
            3 => Ok(LinkStatus::NotAcknowledging),
            _ => Err(Int14hError::UnknownCode {
                field: "link status",
                value,
            }),
        }
    }
}

/// Codes 15 to 19, which come from a Novell LAN driver that no known set ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LanErrorCode(u8);

impl LanErrorCode {
    const FIRST: u8 = 15;
    const LAST: u8 = 19;

    pub const fn code(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for LanErrorCode {
    type Error = Int14hError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (Self::FIRST..=Self::LAST).contains(&value) {
            Ok(LanErrorCode(value))
        } else {
            Err(Int14hError::UnknownCode {
                field: "LAN error",
                value,
            })
        }
    }
}

/// Result of Connect after TSNEXEC maps the driver's `AH` (int14h-api.md, link-layer.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConnectResult {
    /// Connected now or already connected.
    Connected,
    NoDsr,
    CarrierAlwaysDetected,
    BadDialString,
    Busy,
    NoPhoneNetwork,
    NoCarrier,
    /// No `@` prompt from the PAD, often an MNP modem.
    WrongPrompt,
    HostUnreachable,
    LanDriver(LanErrorCode),
    KeyPressed,
}

impl ConnectResult {
    pub const fn code(self) -> u8 {
        match self {
            ConnectResult::Connected => 0,
            ConnectResult::NoDsr => 5,
            ConnectResult::CarrierAlwaysDetected => 6,
            ConnectResult::BadDialString => 7,
            ConnectResult::Busy => 8,
            ConnectResult::NoPhoneNetwork => 9,
            ConnectResult::NoCarrier => 10,
            ConnectResult::WrongPrompt => 12,
            ConnectResult::HostUnreachable => 13,
            ConnectResult::LanDriver(lan) => lan.code(),
            ConnectResult::KeyPressed => 23,
        }
    }
}

impl TryFrom<u8> for ConnectResult {
    type Error = Int14hError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ConnectResult::Connected),
            5 => Ok(ConnectResult::NoDsr),
            6 => Ok(ConnectResult::CarrierAlwaysDetected),
            7 => Ok(ConnectResult::BadDialString),
            8 => Ok(ConnectResult::Busy),
            9 => Ok(ConnectResult::NoPhoneNetwork),
            10 => Ok(ConnectResult::NoCarrier),
            12 => Ok(ConnectResult::WrongPrompt),
            13 => Ok(ConnectResult::HostUnreachable),
            23 => Ok(ConnectResult::KeyPressed),
            _ => LanErrorCode::try_from(value)
                .map(ConnectResult::LanDriver)
                .map_err(|_| Int14hError::UnknownCode {
                    field: "Connect result",
                    value,
                }),
        }
    }
}

/// Result of SwitchHost (int14h-api.md, link-layer.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SwitchHostResult {
    Switched,
    NotConnected,
    HostUnreachable,
    /// The PAD never said `DISCONNECTED`; the client shows a Novell text for this code.
    DisconnectTimeout,
    ReconnectUnsupported,
    LostPhoneNetwork,
    /// The new host refused and the call to the default host was remade.
    Remade,
    /// The UART never reported THRE during the BREAK; leftover arithmetic gives 255.
    BreakFailed,
}

impl SwitchHostResult {
    pub const fn code(self) -> u8 {
        match self {
            SwitchHostResult::Switched => 0,
            SwitchHostResult::NotConnected => 1,
            SwitchHostResult::HostUnreachable => 13,
            SwitchHostResult::DisconnectTimeout => 18,
            SwitchHostResult::ReconnectUnsupported => 20,
            SwitchHostResult::LostPhoneNetwork => 21,
            SwitchHostResult::Remade => 22,
            SwitchHostResult::BreakFailed => 255,
        }
    }
}

impl TryFrom<u8> for SwitchHostResult {
    type Error = Int14hError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(SwitchHostResult::Switched),
            1 => Ok(SwitchHostResult::NotConnected),
            13 => Ok(SwitchHostResult::HostUnreachable),
            18 => Ok(SwitchHostResult::DisconnectTimeout),
            20 => Ok(SwitchHostResult::ReconnectUnsupported),
            21 => Ok(SwitchHostResult::LostPhoneNetwork),
            22 => Ok(SwitchHostResult::Remade),
            255 => Ok(SwitchHostResult::BreakFailed),
            _ => Err(Int14hError::UnknownCode {
                field: "SwitchHost result",
                value,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defined_codes<T, E>(parse: impl Fn(u8) -> Result<T, E>, code: impl Fn(T) -> u8) -> Vec<u8> {
        let parsed = (0..=u8::MAX).filter_map(|value| parse(value).ok().map(|v| (value, v)));
        parsed
            .map(|(value, variant)| {
                assert_eq!(code(variant), value);
                value
            })
            .collect()
    }

    #[test]
    fn codes_round_trip_and_match_the_document() {
        assert_eq!(
            defined_codes(LinkStatus::try_from, LinkStatus::code),
            [0, 1, 2, 3]
        );
        assert_eq!(
            defined_codes(ConnectResult::try_from, ConnectResult::code),
            [0, 5, 6, 7, 8, 9, 10, 12, 13, 15, 16, 17, 18, 19, 23]
        );
        assert_eq!(
            defined_codes(SwitchHostResult::try_from, SwitchHostResult::code),
            [0, 1, 13, 18, 20, 21, 22, 255]
        );
    }
}
