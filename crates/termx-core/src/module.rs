use async_trait::async_trait;

use crate::{Result, Session};

/// Kontext predavany modulu protokolu pri navazovani spojeni.
///
/// Drzi jen to, co potrebuje kazdy protokol spolecne - konkretni moduly
/// si specificke veci (napr. baudrate pro Serial) ctou primo ze `session`
/// nebo ze svych vlastnich rozsirenych poli v budoucnu.
pub struct ConnectionContext<'a> {
    pub session: &'a Session,
}

/// Spolecne rozhrani, ktere musi implementovat kazdy protokolovy modul
/// (termx-ssh, termx-serial, termx-ftp, ...). Aplikace (nyni GUI shell
/// `termx-gui`) pracuje jen s touto abstrakci a nemusi vedet nic o
/// konkretnim protokolu.
///
/// POZNAMKA (GUI pivot): rozhrani zatim pochazi z puvodni TUI verze, kdy
/// modul dostal primo stdin/stdout terminalu v "raw" rezimu. V `termx-gui`
/// zatim neni napojene (viz `render_connection` v `termx-gui`) - az bude
/// vestaveny emulator terminalu (`alacritty_terminal`) skutecne pripojen,
/// bude tato metoda pravdepodobne prepracovana tak, aby misto primeho
/// pristupu ke stdin/stdout cetla/psala do bufferu emulatoru.
#[async_trait]
pub trait ProtocolModule: Send + Sync {
    /// Strojovy identifikator modulu, musi odpovidat [`crate::Protocol::key`].
    fn protocol_key(&self) -> &'static str;

    /// Cloveku srozumitelny nazev modulu (napr. pro seznam dostupnych protokolu).
    fn display_name(&self) -> &'static str;

    /// Naveze spojeni a preda rizeni interaktivni relaci. Vraci se az po
    /// odpojeni (uzivatelem, chybou site apod.).
    async fn run(&self, ctx: ConnectionContext<'_>) -> Result<()>;
}
