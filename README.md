# Term-IX

Terminalovy klient pro spravu vzdalenych spojeni (SSH, seriova linka,
FTP, ...) inspirovany aplikaci MobaXterm - napsany v Rustu, pro
Windows i Linux.

> Stav: **rana faze vyvoje (MVP kostra)**. Funguje SSH modul s
> prihlasenim jmenem/heslem, sifrovane ulozeni serveru, zaklad pro
> self-update a nove i graficke uzivatelske rozhrani (viz nize) -
> vestaveny terminal v nem je zatim jen nahradni obrazovka, skutecne
> pripojeni pres GUI je navazujici krok. Dalsi protokoly a funkce se
> budou pridavat postupne.

## Architektura

Aplikace je rozdelena do samostatnych cargo crates (workspace), aby
zdrojaky nekoncily v jednom nekonecnem souboru a aby slo pridavat
protokoly bez zasahu do zbytku aplikace:

```
Term-IX/
├── src/main.rs          – binarka: propoji vse dohromady, CLI, self-update
└── crates/
    ├── termx-core/       – sdilene typy: Session, AuthMethod, trait
    │                       ProtocolModule, ModuleRegistry, cross-platform
    │                       cesty (AppPaths)
    ├── termx-vault/       – sifrovane ulozeni serveru (AES-256-GCM + Argon2id)
    ├── termx-update/      – self-update z GitHub Releases
    ├── termx-ssh/         – SSH modul (prvni implementace ProtocolModule)
    ├── termx-gui/         – hlavni graficke rozhrani (egui/eframe) - horni
    │                        menu, strom serveru vlevo, taby vpravo
    └── termx-splash/      – uvodni splash okno s logem (mimo terminal)
```

**Pivot z TUI na GUI:** puvodni `termx-tui` (ratatui, terminalove
ovladane textovym rozhranim) byl nahrazen `termx-gui` (egui/eframe,
skutecne graficke okno ve stylu MobaXtermu). Duvod: uzivatel potreboval
administraci vice serveru najednou (strom, slozky, razeni, prejmenovani)
a soubezne otevrene taby vedle sebe, coz je v cistem TUI nepohodlne, a
zejmena aby se terminalovy tab otevrel "prazdny" - bez nativniho okna
OS a jeho vlastniho menu (napr. Windows Terminal profil dropdown), ktere
by prosvitalo skrz. `ModuleRegistry` (registr protokolovych modulu) byl
kvuli tomu presunut z `termx-tui` do `termx-core`, aby ho mohl pouzivat
jak GUI, tak pripadne v budoucnu i jine rozhrani.

### Jak pridat novy protokol (napr. seriovou linku / FTP)

1. Vytvorit novy crate `crates/termx-serial` (nebo `termx-ftp`), pridat
   ho do `members` v korenovem `Cargo.toml`.
2. Implementovat `termx_core::ProtocolModule` (metody `protocol_key`,
   `display_name`, `run`).
3. V `src/main.rs` pridat `registry.register(Arc::new(termx_serial::SerialModule::new()))`.
4. Rozsirit `termx_core::Protocol` o novou variantu (a pripadne
   `AuthMethod`, pokud protokol potrebuje jiny typ prihlaseni).

Zbytek aplikace (GUI, vault, update) se timto nemusi menit.

## Graficke rozhrani (termx-gui)

Po splash obrazovce (viz nize) se rovnou otevre hlavni okno aplikace -
a hned s uvodni "zamcenou" obrazovkou: pole na hlavni heslo trezoru
(pripadne dve pole pro nastaveni hesla, pokud trezor jeste neexistuje),
vse primo v tomto okne. Zadne cmd/konzolove okno se pro zadani hesla
neotevira. Az po uspesnem odemceni/vytvoreni se zobrazi zbytek
rozhrani popsany nize.

Hlavni okno ma tri casti, podobne MobaXtermu:

- **Horni menu** (Terminal / Sessions / View / Tools / Settings / Help)
  - zatim s nejzakladnejsimi polozkami (novy server, nova slozka,
    otevreni Nastaveni jako tabu, ukonceni aplikace); `View`/`Tools`
    jsou pripravene prazdne polozky pro pristi kroky.
- **Levy panel** – strom vsech ulozenych serveru: slozky (i uplne
  prazdne, viz nize) se rozbaluji/sbaluji, kazdy server i slozka maji
  kontextove menu pravym tlacitkem (otevrit, prejmenovat, presunout do
  jine slozky, smazat - slozku jen pokud je prazdna). Strom se staví
  znovu kazdy snimek primo z dat trezoru, takze zadny zvlastni stav
  stromu neni potreba drzet synchronizovany.
- **Pravy hlavni prostor** – lista otevrenych tabu nahore + obsah
  aktivniho tabu. Typy tabu:
  - **Domů** – vzdy pritomny uvodni tab (nejde zavrit).
  - **Nastaveni** – samostatny tab (ne dialogove okenko), otevira se
    pres menu Settings → Predvolby..., chova se stejne jako ostatni
    taby (da se prepnout, zavrit apod.).
  - **Spojeni** (jeden tab na kazdy otevreny server) – **v teto verzi
    je to zatim jen informacni nahradni obrazovka** (jmeno/host/port
    serveru + vysvetlujici text). Skutecny vestaveny emulator
    terminalu (planovane pres `alacritty_terminal`, napojeny na
    `termx_core::ProtocolModule`/`termx-ssh` misto puvodniho primeho
    prevzeti stdin/stdout) je navazujici krok - dulezite je, ze uz
    ted se tab otevira jako cista plocha uvnitr okna aplikace, ne
    jako nativni okno OS s vlastnim menu.

### Slozky ve stromu serveru

`Session.group` drzi cestu k slozce jako retezec se segmenty
oddelenymi lomitkem (napr. `"Prace/PBX"` pro slozku PBX vnorenou v
Praci). Aby mohla existovat i uplne prazdna slozka (pripravena predem,
bez jedineho serveru uvnitr - stejne jako v MobaXtermu), ma
`VaultData` navic pole `folders: Vec<String>` se seznamem takovych cest;
slozka se serverem uvnitr se ve stromu zobrazi i bez zaznamu v tomto
poli. Pole je `#[serde(default)]`, takze starsi trezory (bez tohoto
pole v ulozenem JSONu) se nacitaji beze zmeny.

### Tema

Aktualne existuje jen jedno tmave "terminalove" tema (barvy odvozene z
loga - tmave modro-seda + cyan akcent), viz `termx-gui/src/theme.rs`.
Zamerne se zatim nemenil puvodni vzhled - az bude potreba, pribude v
Nastaveni volba pro druhe, modernejsi tema (`theme.rs` je uz pripraveny
jako samostatny modul prave kvuli tomu).

## Bezpecnost ulozenych serveru

- Ulozene servery (host, port, uzivatel, heslo/cesta ke klici, ...)
  jsou v `termx-vault` serializovany a ulozeny na disk **vzdy
  zasifrovane**: AES-256-GCM, klic odvozeny z hlavniho hesla pomoci
  Argon2id (pomale/pametove narocne KDF - ztezuje brute-force i pri
  uniku souboru).
- Trezor se pri kazdem startu odemyka hlavnim heslem - primo v hlavnim
  okne aplikace (uvodni obrazovka pred zobrazenim stromu/tabu), zadne
  cmd/konzolove okno k tomu neni potreba. Heslo lze kdykoliv zmenit
  pres menu Settings → Změnit heslo trezoru (znovu zasifruje aktualni
  obsah novym heslem). **Kdo hlavni heslo zapomene, k ulozenym udajum
  se uz nedostane** - zadny reset ani "zadni vratka" v aplikaci
  zamerne nejsou.
- Export/import (`Vault::export` / `Vault::import`) umoznuje vytvorit
  samostatny sifrovany soubor (klidne s jinym heslem nez hlavni
  trezor) pro prenos na jiny pocitac - logika hotova v `termx-vault`,
  dialog primo v GUI je jeste navazujici krok (viz roadmap).
- Vychozi umisteni trezoru (odvozeno z nazvu aplikace/organizace pres
  crate `directories`): `%APPDATA%\DaTTcz\Term-IX\data\vault.termx`
  (Windows) / `~/.local/share/term-ix/vault.termx` (Linux).
- **Znamy dluh MVP:** SSH modul zatim neoveruje otisk klice serveru
  (zadny `known_hosts`) - pripoji se k cemukoliv, co odpovi. Doplnit
  pred pouzitim na sitich, kde hrozi MITM.

## Self-update

`termx-update` pri startu (pokud neni pouzit prepinac `--no-update`)
zkontroluje nejnovejsi GitHub Release na
[github.com/DaTTcz/Term-IX](https://github.com/DaTTcz/Term-IX) a
pripadne stahne a nahradi bezici binarku. Release proces
(`.github/workflows/release.yml`) po vytvoreni tagu `vX.Y.Z`
automaticky zabuildi a nahraje binarky pro Windows i Linux.

## Loga a ikony

Zdrojova grafika je v `assets/`:

- `assets/term-ix_logo.png` – plne logo + napis "TERM-IX" (pouzito jako
  splash obrazek).
- `assets/term-ix_ico.png` – samotna znacka (hexagon + X), zdroj pro
  vsechny generovane ikony.
- `assets/icons/term-ix.ico` – multi-rozlisenova Windows ikona
  (16/32/48/64/128/256 px), vygenerovana z `term-ix_ico.png` pres
  ImageMagick (`convert term-ix_ico.png -define icon:auto-resize=... term-ix.ico`).
  `build.rs` ji pri buildu na Windows zabuduje do `term-ix.exe`
  (crate `winres`) - takze exe ma vlastni ikonu v prohlizeci/na hlavnim panelu.
- `assets/icons/hicolor/<velikost>x<velikost>/apps/term-ix.png` –
  stejna znacka v standardni freedesktop.org velikostni rade
  (16 az 512 px) pro instalaci na Linuxu, viz `packaging/term-ix.desktop`.
- `assets/fonts/DejaVuSansMono-Bold.ttf` – monospace font pro "terminalovy"
  vypis verze/autora na splash obrazovce (permisivni Bitstream Vera
  licence, viz `assets/fonts/LICENSE-DejaVu.txt`).

Chcete-li obrazky prehenerovat po zmene loga, staci znovu spustit
prikazy z tohoto oddilu (napr. `convert term-ix_ico.png -resize 128x128 ...`)
pro kazdou pozadovanou velikost.

### Splash obrazovka

Pri startu (pokud neni pouzity prepinac `--no-splash`) se zobrazi
samostatne bezramecke okno s logem. Pod nim se "terminalovym" zpusobem
- pismenko po pismenku, monospace pismem, s kurzorem - vypise
`Term-IX vX.Y.Z` a jmeno autora. Samotne psani (~30 ms/znak) tak
prirozene vytvori kratkou startovaci pauzu bez pocitu, ze se ceka
naprazdno: kurzor je behem psani plny, mezi radky a po dopsani obou
radku par-krat blikne a okno se pak samo zavre (nebo hned po
stisku klavesy/kliknuti). Implementace je v `termx-splash` (`minifb`
pro okno, `fontdue` pro rasterizaci pisma). Teprve po zavreni splash
okna aplikace prevezme terminal a spusti hlavni GUI.

Pokud se graficke okno nepodari otevrit (napr. beh pres SSH bez
X11/Wayland, headless server), splash se tise preskoci a aplikace
pokracuje rovnou do terminaloveho rozhrani - nikdy nesmi zablokovat
spusteni.

## Sestaveni

Vyzaduje Rust (stable) - <https://rustup.rs>.

```sh
cargo build --release
# vysledna binarka: target/release/term-ix(.exe)
```

Spusteni:

```sh
cargo run --release
```

Pri prvnim spusteni aplikace vyzve k nastaveni hlavniho hesla trezoru.

Prepinace: `--no-splash` preskoci uvodni okno s logem, `--no-update`
preskoci kontrolu aktualizaci.

## Dulezita poznamka k tomuto commitu

Tato pocatecni kostra (a naslednych par commitu vcetne GUI pivotu)
byla vytvorena v izolovanem prostredi bez pristupu na crates.io, takze
**zde nebylo mozne spustit `cargo build` / `cargo check`** a kod tak
neprosel automatickym overenim kompilace (zejmena crate `russh` v
`termx-ssh`, `minifb`/`fontdue` v `termx-splash` a nyni `egui`/`eframe`
v `termx-gui` mely/mohou mit v ruznych verzich mirne odlisne API - viz
komentare v prislusnych souborech). Az spustite `cargo build` poprve u
sebe, je mozne, ze bude potreba doladit par nazvu metod/typu.
Architektura (moduly, trait `ProtocolModule`, sifrovani, GUI shell) tim
dotcena neni - jde jen o "posledni mili" prizpusobeni konkretni verzi
zavislosti.

## Roadmap (navrh)

- [ ] Vestaveny emulator terminalu v tabu Spojeni (`alacritty_terminal`),
      napojeny na `ProtocolModule` misto nahradni obrazovky
- [ ] Overeni otisku klice serveru (known_hosts) v `termx-ssh`
- [ ] Prihlaseni SSH privatnim klicem / pres ssh-agent
- [ ] Modul `termx-serial` (seriova linka, obdoba PuTTY/RealTerm)
- [ ] Modul `termx-ftp` / `termx-sftp`
- [ ] Razeni serveru ve stromu (tazenim / rucne), presun tazenim mezi slozkami
- [ ] Export/import trezoru primo z GUI (dialog v Nastaveni - logika v
      `termx-vault` uz existuje)
- [ ] Druhe, modernejsi tema (prepinatelne v Nastaveni)
- [ ] Prenos souboru pretazenim / prikazem v ramci SSH/SFTP relace
- [ ] Kompletni known_hosts + varovani pri zmene klice serveru
