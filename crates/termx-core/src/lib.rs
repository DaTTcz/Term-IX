//! termx-core
//!
//! Sdilene datove typy a rozhrani pro cely Term-IX.
//! Kazdy protokol (SSH, Serial, FTP, ...) je samostatny crate,
//! ktery implementuje trait [`ProtocolModule`] z tohoto crate
//! a registruje se do aplikace jako modul - nova cast kodu se tedy
//! prida jako novy crate, ne jako dalsi funkce v jednom velkem souboru.

mod config;
mod error;
mod module;
mod session;

pub use config::AppPaths;
pub use error::CoreError;
pub use module::{ConnectionContext, ProtocolModule};
pub use session::{AuthMethod, Protocol, Session};

pub type Result<T> = std::result::Result<T, CoreError>;
