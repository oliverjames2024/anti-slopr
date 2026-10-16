#![allow(dead_code)]

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct Person {
    name: String,
}

impl Person {
    fn name(&self) -> &str {
        &self.name
    }
}

struct Employee {
    person: Person,
    salary: u32,
}

impl Deref for Employee {
    type Target = Person;

    fn deref(&self) -> &Person {
        &self.person
    }
}

// Reported once, at the `Deref` impl.
impl DerefMut for Employee {
    fn deref_mut(&mut self) -> &mut Person {
        &mut self.person
    }
}

struct Manager<T> {
    base: Employee,
    reports: Vec<T>,
}

impl<T> Deref for Manager<T> {
    type Target = Employee;

    fn deref(&self) -> &Employee {
        &self.base
    }
}

enum Shape {
    Circle(Person, f64),
    Square(Person, f64),
}

impl Deref for Shape {
    type Target = Person;

    fn deref(&self) -> &Person {
        match self {
            Shape::Circle(person, _) | Shape::Square(person, _) => person,
        }
    }
}

// Not linted: single-field newtypes.
struct Meters(f64);

impl Deref for Meters {
    type Target = f64;

    fn deref(&self) -> &f64 {
        &self.0
    }
}

struct Wrapper {
    inner: Vec<u8>,
}

impl Deref for Wrapper {
    type Target = Vec<u8>;

    fn deref(&self) -> &Vec<u8> {
        &self.inner
    }
}

// Not linted: zero-sized marker fields do not count.
struct Tagged<T> {
    person: Person,
    marker: PhantomData<T>,
    unit: (),
}

impl<T> Deref for Tagged<T> {
    type Target = Person;

    fn deref(&self) -> &Person {
        &self.person
    }
}

// Not linted: targets that are not a concrete struct or enum.
struct Buffer {
    bytes: Vec<u8>,
    len: usize,
}

impl Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

struct Name {
    text: String,
    interned: bool,
}

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

struct Generic<T> {
    value: T,
    version: u32,
}

impl<T> Deref for Generic<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

struct Location {
    path: PathBuf,
    line: u32,
}

impl Deref for Location {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

// Not linted: a guard that implements `Drop` is a smart pointer.
struct Pooled {
    conn: Person,
    pool: Rc<Vec<Person>>,
}

impl Drop for Pooled {
    fn drop(&mut self) {}
}

impl Deref for Pooled {
    type Target = Person;

    fn deref(&self) -> &Person {
        &self.conn
    }
}

// Not linted: the target is held behind a pointer, not embedded.
struct Ref<'a> {
    value: &'a Person,
    borrow_count: u32,
}

impl Deref for Ref<'_> {
    type Target = Person;

    fn deref(&self) -> &Person {
        self.value
    }
}

// Not linted: a `Cow`-like enum where every variant holds just the target.
enum MaybeOwned<'a> {
    Owned(Person),
    Borrowed(&'a Person),
    Shared(Rc<Person>),
}

impl Deref for MaybeOwned<'_> {
    type Target = Person;

    fn deref(&self) -> &Person {
        match self {
            MaybeOwned::Owned(person) => person,
            MaybeOwned::Borrowed(person) => person,
            MaybeOwned::Shared(person) => person,
        }
    }
}

fn main() {
    let employee = Employee {
        person: Person { name: "Ada".into() },
        salary: 1,
    };
    let _ = employee.name();
}
