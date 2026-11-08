//! Typed metadata checks. Password/execution policy and opaque payloads remain owner gates.
use super::{RecoveryModelError, RecoveryRepresentation, validate_range, validate_value};
use crate::{EditRange, PasswordHash, PhoneticProps, PhoneticRun, PhoneticString, PrintSettings, PyCode, PyOutput, SheetProtection, Workbook};
use gridcraft_core::{MAX_COLS, MAX_ROWS, Value};

fn validate_print(print: &PrintSettings) -> Result<(), RecoveryModelError> {
    let PrintSettings {
        margins,
        print_area,
        title_rows,
        title_cols,
        row_breaks,
        col_breaks,
        orientation: _,
        paper: _,
        scale: _,
        fit_to: _,
        gridlines: _,
        headings: _,
        center_h: _,
        center_v: _,
        header: _,
        footer: _,
        different_first: _,
        different_odd_even: _,
        first_header: _,
        first_footer: _,
        even_header: _,
        even_footer: _,
        hf_scale_with_doc: _,
        hf_align_with_margins: _,
        page_order: _,
    } = print;
    if margins.iter().any(|margin| !margin.is_finite() || *margin < 0.0) {
        return Err(RecoveryModelError::Print);
    }
    if let Some(area) = print_area {
        validate_range(*area)?;
    }
    for (titles, breaks, max) in [(title_rows, row_breaks, MAX_ROWS), (title_cols, col_breaks, MAX_COLS)] {
        if titles.is_some_and(|(first, last)| first > last || last >= max) || breaks.iter().any(|&line| line == 0 || line >= max) {
            return Err(RecoveryModelError::Print);
        }
    }
    // Scale/fit are bounded stored integers. Zero fit means automatic; writer clamps or
    // omissions are not model malformation. Break order/duplicates and strings stay exact.
    Ok(())
}

fn retain_password(password: &PasswordHash) {
    // No base64 decoding, algorithm policy, spin-count work or KDF at the model boundary.
    match password {
        PasswordHash::Legacy(_) => {}
        PasswordHash::Iterated { algorithm: _, hash: _, salt: _, spin_count: _ } => {}
    }
}

fn validate_edit_range(range: &EditRange) -> Result<(), RecoveryModelError> {
    let EditRange { title: _, ranges, password, security_descriptor: _ } = range;
    for range in ranges {
        validate_range(*range)?;
    }
    if let Some(password) = password {
        retain_password(password);
    }
    Ok(())
}

fn validate_phonetics(phonetics: &[PhoneticString]) -> Result<(), RecoveryModelError> {
    let count = phonetics.len();
    if count.checked_mul(std::mem::size_of::<&str>()).is_none_or(|bytes| bytes > isize::MAX as usize) {
        return Err(RecoveryModelError::Capacity);
    }
    let mut keys = Vec::new();
    keys.try_reserve_exact(count).map_err(|_| RecoveryModelError::Capacity)?;
    for phonetic in phonetics {
        let PhoneticString { text, runs, props } = phonetic;
        keys.push(text.as_str());
        let units = text.chars().try_fold(0u64, |n, c| n.checked_add(c.len_utf16() as u64).ok_or(RecoveryModelError::Capacity))?;
        for run in runs {
            // Empty reading bytes are retained; only the declared UTF-16 offset invariant applies.
            let PhoneticRun { start, end, text: _ } = run;
            if start >= end || u64::from(*end) > units {
                return Err(RecoveryModelError::Phonetic);
            }
        }
        if let Some(props) = props {
            // Font index is NOT StyleId; unknown type/alignment and props alone are inert.
            let PhoneticProps { font_id: _, kind: _, alignment: _ } = props;
        }
    }
    // Exact TEXT identity, not ASCII folding or Unicode normalization. Comparison time
    // includes the common byte prefixes; the borrowed index allocates no string copies.
    keys.sort_unstable();
    if keys.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(RecoveryModelError::UnsupportedRepresentation(RecoveryRepresentation::AmbiguousIdentity));
    }
    Ok(())
}

/// Count the actual encoder's prefix, syntax, doubled quotes and 255-character chunks.
/// Start with the empty quoted literal; each subsequent chunk adds two quotes and `&`.
/// Stop as soon as the encoded limit is exceeded, without encoding or normalizing code.
fn python_code_fits(code: &str) -> bool {
    let mut encoded = crate::python::FILE_PREFIX.len() + "PY(\"\",0)".len();
    let mut in_chunk = 0usize;
    for c in code.chars() {
        let extra = 1usize + usize::from(c == '"') + if in_chunk == 255 { 3 } else { 0 };
        let Some(next) = encoded.checked_add(extra) else { return false };
        if next > crate::python::MAX_FORMULA_CHARS {
            return false;
        }
        encoded = next;
        if in_chunk == 255 {
            in_chunk = 0;
        }
        in_chunk += 1;
    }
    true
}

pub(super) fn validate_python(py: &PyCode) -> Result<(), RecoveryModelError> {
    let PyCode { code, object: _, output, stale: _ } = py;
    if !python_code_fits(code) {
        return Err(RecoveryModelError::Python);
    }
    if let Some(output) = output {
        let PyOutput { value, type_name: _, preview: _, convertible: _ } = output;
        if let Value::Array(array) = value
            && array.rows.checked_mul(array.cols).is_none_or(|size| size > crate::python::MAX_RESULT_CELLS)
        {
            return Err(RecoveryModelError::Python);
        }
        validate_value(value)?;
    }
    Ok(())
}

pub(super) fn validate_workbook_metadata(workbook: &Workbook) -> Result<(), RecoveryModelError> {
    if let Some(password) = &workbook.structure_password {
        retain_password(password);
    }
    for sheet in &workbook.sheets {
        validate_print(&sheet.print)?;
        if let Some(protection) = &sheet.protection {
            let SheetProtection {
                password,
                legacy_password_hash: _,
                select_locked: _,
                select_unlocked: _,
                format_cells: _,
                format_columns: _,
                format_rows: _,
                insert_columns: _,
                insert_rows: _,
                delete_columns: _,
                delete_rows: _,
                sort: _,
                autofilter: _,
            } = protection;
            if let Some(password) = password {
                retain_password(password);
            }
        }
        for range in &sheet.edit_ranges {
            validate_edit_range(range)?;
        }
    }
    validate_phonetics(&workbook.phonetics)
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
