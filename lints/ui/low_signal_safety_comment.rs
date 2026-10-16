#![allow(unknown_lints, dead_code)]

fn first(slice: &[u32]) -> u32 {
    if slice.is_empty() {
        return 0;
    }

    // SAFETY: this is safe.
    let a = unsafe { *slice.get_unchecked(0) };

    // SAFETY:
    let b = unsafe { *slice.get_unchecked(0) };

    // SAFETY: trust me
    let c = unsafe { *slice.get_unchecked(0) };

    // Safety: checked above.
    let d = unsafe { *slice.get_unchecked(0) };

    // SAFETY: we know this is safe here, trust me.
    let e = unsafe { *slice.get_unchecked(0) };

    // SAFETY: the pointer is always valid and there is no UB.
    let f = unsafe { *slice.as_ptr() };

    /* SAFETY: it's fine */
    let g = unsafe { *slice.as_ptr() };

    // SAFETY: This should be fine because it is safe.
    let h = unsafe { *slice.as_ptr() };

    a + b + c + d + e + f + g + h
}

fn levels(slice: &[u32]) -> u32 {
    #[allow(low_signal_safety_comment)]
    let a = {
        // SAFETY: fine
        unsafe { *slice.as_ptr() }
    };

    a
}

#[expect(low_signal_safety_comment)]
fn expected(slice: &[u32]) -> u32 {
    // SAFETY: trust me
    unsafe { *slice.as_ptr() }
}

// ---------------------------------------------------------------------------
// Not linted.
// ---------------------------------------------------------------------------

fn justified(slice: &[u32], index: usize) -> u32 {
    if index >= slice.len() {
        return 0;
    }

    // Not linted: names the precondition and where it is established.
    // SAFETY: `index < slice.len()` is checked by the early return above.
    let a = unsafe { *slice.get_unchecked(index) };

    // Not linted: the justification continues on the next lines.
    // SAFETY:
    // - `slice` is non-empty because `index < slice.len()`,
    // - so `as_ptr` points to an initialized `u32`.
    let b = unsafe { *slice.as_ptr() };

    // Not linted: a generic opener followed by a real reason.
    // SAFETY: this is safe because the index was bounds-checked above.
    let c = unsafe { *slice.get_unchecked(index) };

    // Not linted: the justification follows after an empty comment line.
    // SAFETY:
    //
    // * `index < slice.len()`, checked by the early return above, so the
    //   element is in bounds and initialized.
    let d = unsafe { *slice.get_unchecked(index) };

    a + b + c + d
}

/// Not linted: a `# Safety` doc section is not a `SAFETY:` comment.
///
/// # Safety
///
/// Safe.
unsafe fn documented(ptr: *const u32) -> u32 {
    // Not linted: the word appears but is not a `SAFETY:` comment.
    // This is the safety net for callers that forget to check.
    unsafe { *ptr }
}

fn string() -> &'static str {
    // Not linted: inside a string literal.
    "// SAFETY: fine"
}

fn main() {}
