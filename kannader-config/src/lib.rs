use std::mem;

use static_assertions::const_assert_eq;

const_assert_eq!(mem::size_of::<usize>(), mem::size_of::<u32>());

pub use kannader_config_types::{implement_guest, server, server_config_implement_guest};
