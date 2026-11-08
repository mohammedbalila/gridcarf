use super::*;
use std::io::Write;
#[test]
fn unicode_scalars_escapes_and_split_utf8_are_measured_without_a_buffer() {
    assert_eq!(measure(&"é", 4, 3).unwrap(), Measure { bytes: 4, chars: 3 });
    assert!(measure(&"é", 3, 3).is_err());
    assert!(measure(&"é", 4, 2).is_err());
    assert_eq!(measure(&"\n", 4, 4).unwrap(), Measure { bytes: 4, chars: 4 });
    let mut count = Counter::new(2, 1);
    count.write_all(&[0xc3]).unwrap();
    count.write_all(&[0xa9]).unwrap();
    assert_eq!(count.measured, Measure { bytes: 2, chars: 1 });
    assert!(count.write_all(b"x").is_err());
    assert!(matches!(count.refusal, Some(RecoveryError::TooLarge(_))));
}
