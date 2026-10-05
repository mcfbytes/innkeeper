//! Golden frames from docs/protocol/link-layer.md, compared byte for byte.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // test helpers may abort

use tsn_link::{encode_frame, Control, Frame, FrameParser, Received, Seq};

const FRAMES: &str = include_str!("golden/frames.txt");

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|pair| u8::from_str_radix(pair, 16).unwrap())
        .collect()
}

fn golden_rows() -> Vec<(Control, Vec<u8>, Vec<u8>)> {
    let rows = FRAMES
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty());
    rows.map(|line| {
        let fields: Vec<&str> = line.split('|').collect();
        let (kind, seq) = fields[0].trim().split_once(' ').unwrap();
        let seq = Seq::try_from(seq.parse::<u8>().unwrap()).unwrap();
        let control = match kind {
            "ack" => Control::Ack(seq),
            "nak" => Control::Nak(seq),
            "data" => Control::Data(seq),
            other => panic!("unknown frame type {other}"),
        };
        (control, hex(fields[1]), hex(fields[2]))
    })
    .collect()
}

#[test]
fn encoder_reproduces_every_golden_frame() {
    let rows = golden_rows();
    assert_eq!(rows.len(), 22);
    for (control, payload, wire) in rows {
        assert_eq!(encode_frame(control, &payload).unwrap(), wire, "{control}");
    }
}

#[test]
fn parser_decodes_the_golden_frames_back_to_back() {
    let rows = golden_rows();
    let stream: Vec<u8> = rows.iter().flat_map(|(_, _, wire)| wire.clone()).collect();
    let mut parser = FrameParser::new();
    let decoded: Vec<Received> = stream
        .iter()
        .filter_map(|&byte| parser.push(byte))
        .collect();
    let expected: Vec<Received> = rows
        .into_iter()
        .map(|(control, payload, _)| Received::Frame(Frame { control, payload }))
        .collect();
    assert_eq!(decoded, expected);
}
