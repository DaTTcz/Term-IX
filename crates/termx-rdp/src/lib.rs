//! termx-rdp
//!
//! RDP (Remote Desktop Protocol) protokolovy modul - pripojeni ke
//! vzdalene plose Windows. Stejny pristup jako `termx-ftp`/`termx-serial`:
//! `ironrdp-blocking` je synchronni (blokujici) knihovna, takze zadny
//! tokio runtime tu neni potreba - skutecne spojeni bezi ve vlastnim OS
//! vlakne a komunikuje s GUI pres obycejne `std::sync::mpsc` kanaly
//! (`RdpCommand`/`RdpEvent`, viz `rdp.rs`).
//!
//! Na rozdil od `termx-ftp`/`sftp_browser.rs` (soubory) tenhle modul
//! nezobrazuje sve UI primo v tabu - vestavena plocha bezi v SAMOSTATNEM
//! OS okne (egui "deferred viewport"), tab jen ukazuje stavovou kartu.
//! Viz `termx-gui::rdp_viewer` a diskuze v `claude/roadmap-ideas.md`
//! (Cowork projekt), proc: uzivatel chtel jit nezavisle
//! maximalizovat/minimalizovat plochu vzdaleneho stolu, ne mit ji
//! zamcenou uvnitr tabu hlavniho okna aplikace.
//!
//! ROZSAH PRVNI VERZE (viz i header komentar `rdp.rs`): funkcni spojeni +
//! zobrazeni plochy + mys/klavesnice. Schranka (clipboard sync) je
//! ZAMERNE VYNECHANA - priorita byla "nejdřív funkční spojení", schranka
//! pribude pozdeji jako samostatny krok (vyzaduje dalsi IronRDP
//! subsystem, `ironrdp-cliprdr`).

mod rdp;

use termx_core::{ConnectionContext, ProtocolModule};

pub use rdp::{
    spawn_rdp_session, DirtyRect, RdpCommand, RdpConnectParams, RdpEvent, RdpHandle, RdpMouseButton, RdpWake, RdpWaker,
    SharedFrame,
};

#[derive(Default)]
pub struct RdpModule;

impl RdpModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ProtocolModule for RdpModule {
    fn protocol_key(&self) -> &'static str {
        "rdp"
    }

    fn display_name(&self) -> &'static str {
        "RDP (Vzdálená plocha)"
    }

    /// Stejne jako u `FtpModule`/`SerialModule`/`SshModule::run` - tenhle
    /// obecny (mimo-GUI) beh se aktualne nepouziva, GUI (`termx-gui`) se
    /// napojuje primo na [`spawn_rdp_session`] (viz `rdp_viewer.rs`).
    async fn run(&self, _ctx: ConnectionContext<'_>) -> termx_core::Result<()> {
        Err(termx_core::CoreError::Module(
            "samostatný (mimo GUI) režim RDP modulu zatím není implementován".into(),
        ))
    }
}
