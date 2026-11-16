//! The app's data source. Every read is a `source` declared in `app.contract`, which the runner
//! answers, so this answers nothing. It exists because a native build without `app.ts` needs a
//! Rust data crate (app.json `rust.data`); otherwise it bakes TypeScript and refuses the app.

use exact_runner::{DataError, DataSource, Value};

/// The app's data source.
#[derive(Default, Clone, Copy)]
pub struct Credits;

impl DataSource for Credits {
    fn query(&mut self, source: &str, _: &[Value]) -> Result<Value, DataError> {
        Err(DataError::UnknownSource(source.into()))
    }
}
