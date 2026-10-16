// Compiled with `--test`, so `#[test]` and `#[cfg(test)]` code is present.
#![allow(dead_code)]

use std::collections::HashMap;

fn expect_option(values: &[u8]) -> u8 {
    *values.first().expect("failed")
}

fn expect_result(input: &str) -> u16 {
    input.parse().expect("should never happen")
}

fn expect_short(map: &HashMap<&str, u8>) -> u8 {
    map["a"] + *map.get("b").expect("missing b")
}

fn expect_generic_phrase(input: &str) -> u16 {
    input.parse().expect("This should never happen!")
}

fn expect_err(input: &str) -> std::num::ParseIntError {
    input.parse::<u8>().expect_err("must fail")
}

fn expect_empty(values: &[u8]) -> u8 {
    *values.last().expect("")
}

fn panic_literal(code: u8) -> u8 {
    if code > 9 {
        panic!("oops");
    }
    code
}

fn unreachable_literal(code: u8) -> u8 {
    match code {
        0 => 1,
        _ => unreachable!("impossible"),
    }
}

fn panic_generic(code: u8) {
    if code == 0 {
        panic!("Something went wrong.");
    }
}

fn panic_raw(code: u8) {
    if code == 1 {
        panic!(r#"bad state"#);
    }
}

// Not linted: messages that state the invariant.
fn good_messages(values: &[u8], input: &str) -> u16 {
    let first = values.first().expect("`values` is checked to be non-empty by the caller");
    let parsed: u16 = input.parse().expect("input was validated as digits only");
    if *first == 0 {
        unreachable!("zero is filtered out before this point");
    }
    if parsed == 0 {
        panic!("port 0 is reserved and rejected during validation");
    }
    parsed
}

// Not linted: messages with format arguments carry runtime context.
fn formatted(code: u8) {
    if code == 2 {
        panic!("bad {code}");
    }
    if code == 3 {
        unreachable!("code {}", code);
    }
}

// Not linted: non-literal messages, bare macros, `todo!`, and `assert!`.
fn non_literal(values: &[u8], reason: &str, code: u8) -> u8 {
    assert!(code < 200, "bad");
    if code == 4 {
        todo!("later");
    }
    if code == 5 {
        unreachable!();
    }
    *values.first().expect(reason)
}

struct Builder;

impl Builder {
    // Not linted: a user-defined `expect` that is not `Option`/`Result`.
    fn expect(&self, _token: &str) -> &Self {
        self
    }
}

macro_rules! must {
    ($e:expr) => {
        $e.expect("failed")
    };
}

// Not linted: produced by a macro expansion.
fn from_macro(values: &[u8]) -> u8 {
    *must!(values.first())
}

// Not linted: test code.
#[test]
fn in_test() {
    let value: Option<u8> = Some(1);
    assert_eq!(value.expect("failed"), 1);
}

#[cfg(test)]
mod tests {
    fn helper() -> u8 {
        "1".parse().expect("oops")
    }
}

fn main() {
    let _ = (expect_option(&[1]), expect_result("1"), expect_generic_phrase("1"), expect_err("x"));
    let _ = (expect_short(&HashMap::new()), expect_empty(&[1]), panic_literal(1), unreachable_literal(0));
    panic_generic(1);
    panic_raw(0);
    formatted(0);
    let _ = (good_messages(&[1], "1"), non_literal(&[1], "x", 0), from_macro(&[1]));
    let _ = Builder.expect("x").expect("y");
}
