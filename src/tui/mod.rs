mod app;
#[cfg(feature = "bench")]
pub(crate) mod bench_support;
mod clipboard;
mod editor_highlight;
mod editor_state;
mod entry_rows;
mod env_strip;
mod environment;
mod errors;
mod events;
mod features;
mod geocode;
mod hit_test;
mod image;
mod render;
mod runtime;
mod scroll;
mod search;
mod state;
mod surface;
mod syntax_highlight;
#[cfg(test)]
mod test_support;
mod text_input;
pub(crate) mod theme;
mod ui;

pub(crate) use errors::concise_error;
pub(crate) use runtime::{run, run_compose};
