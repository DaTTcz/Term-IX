// Vklada ikonu aplikace (assets/icons/term-ix.ico) do vysledneho .exe -
// aby Windows Prohlizec/hlavni panel/Alt+Tab ukazovaly logo Term-IX misto
// vychoziho "prazdneho" konzoloveho okna. Na jinych OS se tento krok
// preskoci (`winres` je zavislosti jen pro `cfg(windows)`, viz Cargo.toml).
fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icons/term-ix.ico");
        if let Err(e) = res.compile() {
            // Nechceme kvuli chybejici/vadne ikone shodit cely build -
            // jen na to upozornime.
            println!("cargo:warning=nepodarilo se zabudovat ikonu do .exe: {e}");
        }
    }
}
