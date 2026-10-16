use std::borrow::Cow;
use std::num::ParseIntError;

fn owned(input: &str) -> Result<u16, String> {
    input.parse().map_err(|err| format!("invalid port: {err}"))
}

fn borrowed(input: &str) -> Result<u16, &'static str> {
    input.parse().map_err(|_| "invalid port")
}

fn boxed(input: &str) -> Result<u16, Box<str>> {
    input.parse().map_err(|_| "invalid port".into())
}

fn cow(input: &str) -> Result<u16, Cow<'static, str>> {
    input.parse().map_err(|_| Cow::Borrowed("invalid port"))
}

type StringResult<T> = Result<T, String>;

fn aliased() -> StringResult<()> {
    Ok(())
}

trait Loader {
    fn load(&self) -> Result<Vec<u8>, String>;
}

struct Disk;

// Not linted: the trait declaration owns the signature.
impl Loader for Disk {
    fn load(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

// Not linted: real error types.
fn typed(input: &str) -> Result<u16, ParseIntError> {
    input.parse()
}

// Not linted: string success values are fine.
fn string_ok() -> Result<String, std::io::Error> {
    Ok(String::new())
}

fn main() {
    let _ = (owned, borrowed, boxed, cow, aliased, typed, string_ok);
    let _ = Disk.load();
}
