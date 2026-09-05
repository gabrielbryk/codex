use super::*;

#[test]
fn accepts_one_bounded_row_and_rejects_unsafe_output() {
    pretty_assertions::assert_eq!(parse_output("ready ✓\n".as_bytes()), Ok("ready ✓".into()));
    assert!(parse_output(b"one\ntwo").is_err());
    assert!(parse_output(b"\x1b[31mred").is_err());
    assert!(parse_output(&[0xff]).is_err());
    assert!(parse_output(&vec![b'x'; super::MAX_OUTPUT_BYTES + 1]).is_err());
    assert!(parse_output("界".repeat(129).as_bytes()).is_err());
}
