use super::RecoveryError;
use serde::Serialize;
use std::io::{self, Write};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Measure {
    pub(super) bytes: usize,
    pub(super) chars: usize,
}
struct Counter {
    measured: Measure,
    bytes: usize,
    chars: usize,
    refusal: Option<RecoveryError>,
}
impl Counter {
    fn new(bytes: usize, chars: usize) -> Self {
        Self { measured: Measure::default(), bytes, chars, refusal: None }
    }
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.refusal.is_some() {
            return Err(io::Error::other("recovery character refusal"));
        }
        let result = (|| {
            super::charge(&mut self.measured.bytes, bytes.len(), self.bytes, "manifest")?;
            // serde emits valid UTF-8 overall; a scalar's leading byte is counted once even across chunks.
            super::charge(&mut self.measured.chars, bytes.iter().filter(|b| **b & 0xc0 != 0x80).count(), self.chars, "macro_chars")
        })();
        match result {
            Ok(()) => Ok(bytes.len()),
            Err(error) => {
                self.refusal = Some(error);
                Err(io::Error::other("recovery character refusal"))
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn measure<T: Serialize + ?Sized>(value: &T, bytes: usize, chars: usize) -> Result<Measure, RecoveryError> {
    let mut counter = Counter::new(bytes, chars);
    let result = serde_json::to_writer(&mut counter, value);
    if let Some(error) = counter.refusal {
        return Err(error);
    }
    result.map_err(|_| RecoveryError::Malformed("macro_writer"))?;
    Ok(counter.measured)
}

#[cfg(test)]
mod tests;
