use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::num::ParseIntError;

fn to_string(input: &str) -> Result<u16, Box<dyn std::error::Error>> {
    let port: u16 = input.parse().map_err(|e: ParseIntError| e.to_string())?;
    Ok(port)
}

fn formatted<T: std::str::FromStr>(input: &str) -> Result<T, Box<dyn std::error::Error>>
where
    T::Err: fmt::Display,
{
    Ok(input.parse::<T>().map_err(|e| format!("invalid value {input:?}: {e}"))?)
}

fn static_str(input: &str) -> Option<u16> {
    let parsed: Result<u16, &str> = input.parse::<u16>().map_err(|_| "bad port");
    parsed.ok()
}

fn fn_path(input: &str) -> Result<u16, String> {
    input.parse::<u16>().map_err(|e| e.to_string())
}

fn to_string_path(input: &str) -> Result<i64, Box<str>> {
    input.parse::<i64>().map_err(|e| e.to_string().into_boxed_str())
}

fn tostring_fn(input: &str) -> Result<i64, Cow<'static, str>> {
    input.parse::<i64>().map_err(|e| Cow::Owned(ToString::to_string(&e)))
}

fn describe(err: ParseIntError) -> String {
    format!("not a number: {err}")
}

fn fn_item(input: &str) -> Result<u8, Box<dyn std::error::Error>> {
    Ok(input.parse::<u8>().map_err(describe)?)
}

fn ok_or_literal(map: &HashMap<u32, String>, id: u32) -> Result<&String, Box<dyn std::error::Error>> {
    Ok(map.get(&id).ok_or("no such user")?)
}

fn ok_or_format(map: &HashMap<u32, String>, id: u32) -> Result<&String, Box<dyn std::error::Error>> {
    Ok(map.get(&id).ok_or(format!("no user {id}"))?)
}

fn ok_or_else_format(map: &HashMap<u32, String>, id: u32) -> Result<&String, Box<dyn std::error::Error>> {
    Ok(map.get(&id).ok_or_else(|| format!("no user {id}"))?)
}

#[derive(Debug)]
enum ConfigError {
    InvalidPort(ParseIntError),
    Missing(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPort(source) => write!(f, "invalid port: {source}"),
            Self::Missing(key) => write!(f, "missing {key}"),
        }
    }
}

impl std::error::Error for ConfigError {}

// Not linted: `map_err` into a real error type keeps the cause.
fn typed(input: &str) -> Result<u16, ConfigError> {
    input.parse().map_err(ConfigError::InvalidPort)
}

// Not linted: `ok_or` with a real error type.
fn typed_ok_or(map: &HashMap<u32, u16>, id: u32) -> Result<u16, ConfigError> {
    map.get(&id).copied().ok_or(ConfigError::Missing("port"))
}

// Not linted: the error is already a string, nothing is lost at this call.
fn already_string(result: Result<u16, String>) -> Result<u16, String> {
    result.map_err(|e| format!("while loading: {e}"))
}

// Not linted: `map` over the success value produces a string, not the error.
fn string_ok(input: &str) -> Result<String, ParseIntError> {
    input.parse::<u16>().map(|port| port.to_string())
}

// Not linted: `anyhow::Context` keeps the source chain.
fn with_context(input: &str) -> anyhow::Result<u16> {
    use anyhow::Context;
    input.parse::<u16>().context("invalid port")
}

struct Future;

impl Future {
    // Not linted: a user-defined `map_err` on a non-`Result` type.
    fn map_err<F: FnOnce(()) -> String>(self, f: F) -> Result<(), String> {
        Err(f(()))
    }
}

macro_rules! stringify_err {
    ($e:expr) => {
        $e.map_err(|e: ParseIntError| e.to_string())
    };
}

// Not linted: produced by a macro expansion.
fn from_macro(input: &str) -> Option<u16> {
    stringify_err!(input.parse::<u16>()).ok()
}

fn main() {
    let map = HashMap::new();
    let ports = HashMap::new();
    let _ = (to_string("1"), formatted::<u8>("1"), static_str("1"), fn_path("1"), to_string_path("1"));
    let _ = (tostring_fn("1"), fn_item("1"), ok_or_literal(&map, 1), ok_or_format(&map, 1));
    let _ = (ok_or_else_format(&map, 1), typed("1"), typed_ok_or(&ports, 1), already_string(Ok(1)));
    let _ = (string_ok("1"), with_context("1"), Future.map_err(|()| String::new()), from_macro("1"));
}
