use crate::crc::Crc16;
use crate::{Control, LinkError};

pub(crate) const SOF: u8 = 0x81;
pub(crate) const EOF: u8 = 0x82;
pub(crate) const ESC: u8 = 0x1B;

/// SOF, two CRC bytes, `ctrl` and EOF.
const FRAME_OVERHEAD: usize = 5;
/// TSNEXEC's packer keeps a whole frame, SOF to EOF, within 254 bytes (`0000:2685`).
const MAX_FRAME_LEN: usize = 0xFE;
/// Largest payload a frame can carry, counted after escaping.
pub(crate) const MAX_ESCAPED_PAYLOAD: usize = MAX_FRAME_LEN - FRAME_OVERHEAD;

/// One decoded frame; `payload` is unescaped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub control: Control,
    pub payload: Vec<u8>,
}

pub(crate) const fn needs_escape(byte: u8) -> bool {
    matches!(byte, SOF | EOF | ESC)
}

pub(crate) const fn escaped_len(byte: u8) -> usize {
    if needs_escape(byte) {
        2
    } else {
        1
    }
}

/// Bytes the payload occupies on the wire once escaped.
pub(crate) fn escaped_size(payload: &[u8]) -> usize {
    payload.iter().map(|&byte| escaped_len(byte)).sum()
}

/// Encodes one frame: escapes the payload, then covers `ctrl` and the escaped bytes with the CRC.
pub fn encode_frame(control: Control, payload: &[u8]) -> Result<Vec<u8>, LinkError> {
    let escaped = escaped_size(payload);
    if escaped > MAX_ESCAPED_PAYLOAD {
        return Err(LinkError::PayloadTooLarge { escaped });
    }
    Ok(assemble_frame(control, payload, escaped))
}

/// Encodes a frame without payload, the only form a host may use for ACK and NAK.
pub(crate) fn encode_control_frame(control: Control) -> Vec<u8> {
    assemble_frame(control, &[], 0)
}

fn assemble_frame(control: Control, payload: &[u8], escaped: usize) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + escaped);
    body.push(control.to_byte());
    for &byte in payload {
        if needs_escape(byte) {
            body.push(ESC);
        }
        body.push(byte);
    }
    let mut frame = Vec::with_capacity(body.len() + FRAME_OVERHEAD - 1);
    frame.push(SOF);
    frame.extend(Crc16::over(&body).to_le_bytes());
    frame.extend(body);
    frame.push(EOF);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Seq;

    #[test]
    fn payload_bytes_that_collide_with_framing_are_escaped() {
        let frame = encode_frame(Control::Data(Seq::ZERO), &[SOF, 0x41, ESC, EOF]).unwrap();
        assert_eq!(
            &frame[4..frame.len() - 1],
            &[ESC, SOF, 0x41, ESC, ESC, ESC, EOF]
        );
    }

    #[test]
    fn escaped_size_limit_is_enforced() {
        assert!(encode_frame(Control::Data(Seq::ZERO), &[0x41; MAX_ESCAPED_PAYLOAD]).is_ok());
        let escaped_too_long = [SOF; MAX_ESCAPED_PAYLOAD / 2 + 1];
        assert_eq!(
            encode_frame(Control::Data(Seq::ZERO), &escaped_too_long),
            Err(LinkError::PayloadTooLarge { escaped: 250 })
        );
    }

    #[test]
    fn control_frame_matches_general_encoder() {
        let ack = Control::Ack(Seq::ZERO);
        assert_eq!(encode_control_frame(ack), encode_frame(ack, &[]).unwrap());
    }
}
