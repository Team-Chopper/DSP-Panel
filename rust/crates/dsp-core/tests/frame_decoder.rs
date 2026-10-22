//! Port of packages/dsp_core/test/frame_decoder_test.dart.

use dsp_core::FrameDecoder;
use dsp_core::protocol::{calculate_checksum, hex_to_bytes};
use dsp_core::testing::{CONFIG_DUMP_FRAMES, captured_frames};

fn response(cmd: u8, payload: &[u8]) -> Vec<u8> {
    let data = [&[0x01, 0x00, payload.len() as u8 + 1, cmd][..], payload].concat();
    [&[0x10, 0x02][..], &data, &[0x10, 0x03, calculate_checksum(&data)]].concat()
}

#[test]
fn captured_frames_satisfy_the_length_and_checksum_rule() {
    for frame in captured_frames() {
        assert_eq!(frame.len(), frame[4] as usize + 8);
        assert_eq!(FrameDecoder::new().add(&frame), vec![frame.clone()]);
    }
}

#[test]
fn several_frames_in_one_read() {
    let frames = captured_frames();
    let mut decoder = FrameDecoder::new();
    assert_eq!(decoder.add(&frames.concat()), frames);
    assert_eq!(decoder.dropped_bytes, 0);
}

#[test]
fn frames_split_into_single_bytes() {
    let frames: Vec<Vec<u8>> = captured_frames().into_iter().take(5).collect();
    let mut decoder = FrameDecoder::new();
    let mut out = Vec::new();
    for b in frames.concat() {
        out.extend(decoder.add(&[b]));
    }
    assert_eq!(out, frames);
}

#[test]
fn payload_containing_10_03_is_not_cut_short() {
    let frame = response(0x24, &[0x01, 0x10, 0x03, 0x42, 0x10, 0x03, 0x00, 0x99]);
    let mut decoder = FrameDecoder::new();
    assert!(decoder.add(&frame[..9]).is_empty());
    assert_eq!(decoder.add(&frame[9..]), vec![frame.clone()]);
}

#[test]
fn payload_containing_10_02_is_not_mistaken_for_a_frame_start() {
    let frame = response(0x24, &[0x02, 0x10, 0x02, 0x00, 0x01, 0x05]);
    assert_eq!(FrameDecoder::new().add(&frame), vec![frame.clone()]);
}

#[test]
fn garbage_before_a_frame_is_skipped() {
    let frame = captured_frames().remove(0);
    let mut decoder = FrameDecoder::new();
    let input = [&[0x00, 0xff, 0x10, 0x55][..], &frame].concat();
    assert_eq!(decoder.add(&input), vec![frame]);
    assert_eq!(decoder.dropped_bytes, 4);
}

#[test]
fn corrupted_frame_is_dropped_and_the_decoder_resynchronises() {
    let frames = captured_frames();
    let mut broken = frames[0].clone();
    broken[20] ^= 0xff;
    let mut decoder = FrameDecoder::new();
    assert_eq!(decoder.add(&[broken, frames[1].clone()].concat()), vec![frames[1].clone()]);
    assert!(decoder.dropped_bytes > 0);
}

#[test]
fn frame_with_a_wrong_length_byte_is_recovered_by_its_footer() {
    // Two payload bytes too many for its length byte (like the transcription
    // error that was in analyze_pcap.py), with a matching checksum.
    let original = hex_to_bytes(CONFIG_DUMP_FRAMES[7]).unwrap();
    let mut corrupted = [&original[..9], &[0x78, 0x00], &original[9..original.len() - 1]].concat();
    let checksum = calculate_checksum(&corrupted[2..corrupted.len() - 2]);
    corrupted.push(checksum);
    let first = captured_frames().remove(0);
    let mut decoder = FrameDecoder::new();
    assert_eq!(decoder.add(&[corrupted.clone(), first.clone()].concat()), vec![corrupted, first]);
    assert_eq!(decoder.length_mismatches, 1);
}
