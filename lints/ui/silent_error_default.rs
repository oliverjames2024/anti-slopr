// The patterns below are deliberate; clippy would suggest other forms of them.
#![allow(clippy::unnecessary_lazy_evaluations, clippy::lines_filter_map_ok)]

use std::env;
use std::fs;
use std::io::{self, BufRead};
use std::num::ParseIntError;

fn parse_port(input: &str) -> u16 {
    input.parse().unwrap_or(8080)
}

fn parse_count(input: &str) -> u32 {
    input.parse().unwrap_or_default()
}

fn wildcard_closure(input: &str) -> u32 {
    input.parse().unwrap_or_else(|_| 0)
}

fn unused_binding(input: &str) -> u32 {
    input.parse().unwrap_or_else(|_err: ParseIntError| 1)
}

fn ok_then_default(path: &str) -> String {
    fs::read_to_string(path).ok().unwrap_or_default()
}

fn ok_then_unwrap_or(input: &str) -> bool {
    input.parse().ok().unwrap_or(false)
}

fn ok_then_flatten(result: Result<Option<u8>, io::Error>) -> Option<u8> {
    result.ok().flatten()
}

fn filter_map_path(lines: impl BufRead) -> Vec<String> {
    lines.lines().filter_map(Result::ok).collect()
}

fn filter_map_closure(inputs: &[&str]) -> Vec<u8> {
    inputs.iter().map(|input| input.parse::<u8>()).filter_map(|parsed| parsed.ok()).collect()
}

fn flat_map_path(inputs: &[&str]) -> Vec<u8> {
    inputs.iter().map(|input| input.parse::<u8>()).flat_map(Result::ok).collect()
}

fn flatten_results(dir: &str) -> io::Result<usize> {
    Ok(fs::read_dir(dir)?.flatten().count())
}

// Not linted: the error is propagated.
fn propagated(input: &str) -> Result<u16, ParseIntError> {
    input.parse()
}

// Not linted: the error binding is used (logged) in the fallback.
fn logged(input: &str) -> u16 {
    input.parse().unwrap_or_else(|err| {
        eprintln!("invalid port {input:?}: {err}");
        8080
    })
}

// Not linted: the fallback closure diverges.
fn diverges(input: &str) -> u16 {
    input.parse().unwrap_or_else(|_| std::process::exit(2))
}

// Not linted: a named function receives the error.
fn handler(input: &str) -> u16 {
    fn on_error(err: ParseIntError) -> u16 {
        eprintln!("{err}");
        0
    }
    input.parse().unwrap_or_else(on_error)
}

// Not linted: `Option` methods.
fn option_methods(values: &[u8], nested: Option<Option<u8>>) -> u8 {
    let first = values.first().copied().unwrap_or_default();
    let last = values.last().copied().unwrap_or(0);
    first + last + nested.flatten().unwrap_or_else(|| 1)
}

// Not linted: iterators of `Option`s, and `filter_map` with another function.
fn option_iterators(values: &[Option<u8>], inputs: &[&str]) -> usize {
    let present = values.iter().flatten().count();
    let filtered = values.iter().filter_map(|value| *value).count();
    let parsed = inputs.iter().filter_map(|input| input.parse::<u8>().ok().map(|_| ())).count();
    present + filtered + parsed
}

// Not linted: `map_while(Result::ok)` stops at the first error instead of skipping it.
fn map_while(lines: impl BufRead) -> Vec<String> {
    lines.lines().map_while(Result::ok).collect()
}

// Not linted: `VarError` and `TryFromIntError` only mean "absent" / "does not fit".
fn allowed_errors(wide: u64) -> (String, u32) {
    (env::var("HOME").unwrap_or_default(), wide.try_into().unwrap_or(u32::MAX))
}

// Not linted: `.ok()` that is propagated with `?` in an `Option` context.
fn ok_question(input: &str) -> Option<u16> {
    let port: u16 = input.parse().ok()?;
    Some(port)
}

macro_rules! lenient {
    ($e:expr) => {
        $e.unwrap_or_default()
    };
}

// Not linted: produced by a macro expansion.
fn from_macro(input: &str) -> u8 {
    lenient!(input.parse::<u8>())
}

fn main() {
    let stdin = io::stdin();
    let _ = (parse_port("1"), parse_count("1"), wildcard_closure("1"), unused_binding("1"));
    let _ = (ok_then_default("x"), ok_then_unwrap_or("true"), ok_then_flatten(Ok(None)));
    let _ = (filter_map_path(stdin.lock()), filter_map_closure(&[]), flat_map_path(&[]), flatten_results("."));
    let _ = (propagated("1"), logged("1"), diverges("1"), handler("1"), option_methods(&[], None));
    let _ = (option_iterators(&[], &[]), map_while(stdin.lock()), allowed_errors(1), ok_question("1"));
    let _ = from_macro("1");
}
