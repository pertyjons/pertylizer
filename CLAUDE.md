# CLAUDE.md - Projektinstruktioner för modular-synth

## Meta: Uppdatera detta dokument

När användaren anger nya krav, kodstilsregler, eller arbetssätt som ska gälla framöver:
1. Lägg till dem i relevant sektion i denna fil (CLAUDE.md)
2. Bekräfta ändringen för användaren
3. Om kravet inte passar någon befintlig sektion, skapa en ny

## Projektfas

Vi är i aktiv utveckling - **ingen bakåtkompatibilitet krävs**. Bryt API:er fritt för att förbättra koden.

---

## Beroenden och versioner

### Rust-version
Använd alltid senaste stabila Rust-version. Uppdatera `rust-toolchain.toml` om den finns, eller skapa en:
```toml
[toolchain]
channel = "stable"
```

### Cargo-beroenden
Håll alla beroenden uppdaterade till senaste version. Kör regelbundet:
```bash
# Uppdatera alla beroenden till senaste kompatibla version
cargo update

# Kontrollera om det finns nyare major-versioner
cargo outdated
```

Vid `uppdatera version` eller när du arbetar med beroenden:
1. Kör `cargo update` för att uppdatera Cargo.lock
2. Kör `cargo outdated` (om installerat) för att se om det finns nyare major-versioner
3. Uppdatera Cargo.toml manuellt för major-versioner om det behövs
4. Verifiera att allt kompilerar och tester passerar

---

## Git-kommandon

### `git commit`
Lägg till alla filer (nya och ändrade) och committa med kort beskrivning:
```bash
git add --all
git commit -m "<kort beskrivning av ändringarna>"
```

### `uppdatera version`
1. Uppdatera `docs/history.md` med nytt versionsnummer och ändringar sedan senaste versionen
2. Granska `docs/TODO.md` och markera avklarade uppgifter som klara

---

## Kodstil och mönster

### Newtype-mönstret (obligatoriskt)

Använd ALLTID typade domänvärden - **aldrig råa primitiver** som `f32`, `u8`, `usize` för domänkoncept.

Referens: https://doc.rust-lang.org/rust-by-example/generics/new_types.html

```rust
// FEL:
fn set_frequency(hz: f32) { ... }
fn set_cutoff(hz: f32) { ... }  // Lätt att blanda ihop!

// RÄTT:
fn set_frequency(freq: Hertz) { ... }
fn set_cutoff(cutoff: Hertz) { ... }  // Typsäkert
```

Befintliga typer att använda:
- **Frekvens:** `Hertz`
- **Amplitud:** `Gain`, `Decibels`
- **Tid:** `Seconds`, `Milliseconds`, `Bpm`, `BeatDivision`
- **Normaliserat:** `NormalizedValue` (0.0-1.0), `BipolarValue` (-1.0 till 1.0), `Phase`
- **MIDI:** `MidiNote`, `MidiChannel`
- **Samples:** `SampleCount`, `SamplePosition`, `SampleRate`, `BufferIndex`
- **DSP:** `FilterState`, `NoiseState`

### Namnkonventioner

- **Typer:** `PascalCase` - `Hertz`, `NormalizedValue`
- **Funktioner/metoder:** `snake_case` - `to_frequency()`, `as_f32()`
- **Konstanter:** `SCREAMING_SNAKE_CASE` - `Hertz::A4`, `Gain::UNITY`
- **Använd `Self`** i impl-block, inte typnamnet

---

## Kompilering och kodkvalitet

### Obligatoriska kontroller

Innan en uppgift anses klar MÅSTE följande passera utan varningar eller fel:

```bash
# Steg 1: Kompilera med alla varningar som fel
RUSTFLAGS="-D warnings" cargo build --release

# Steg 2: Clippy - standardlints som fel, pedantic som varningar
cargo clippy --all-targets -- \
    -D warnings \
    -D clippy::unwrap_used \
    -D clippy::expect_used \
    -D clippy::panic \
    -W clippy::pedantic \
    -W clippy::nursery

# Steg 3: Kör alla tester
cargo test

# Steg 4: Kontrollera formatering
cargo fmt --check
```

**Notera:** Pedantic/nursery lints är `-W` (varningar) inte `-D` (fel). De är vägledande
men behöver inte åtgärdas om de inte är relevanta. Undantag definieras i `lib.rs`.

### Strikta regler

1. **Inga `.unwrap()` eller `.expect()`** i produktionskod - använd `unwrap_or`, `unwrap_or_default`, `?`, eller `if let`
2. **Ingen `unsafe` kod** - diskutera först om absolut nödvändigt
3. **`#[must_use]`** på viktiga pub-funktioner där returvärdet inte bör ignoreras (builders, transformers)
4. **Lokala `#[allow(...)]`** är OK för godkända undantag (se nedan) - kräver inte godkännande
5. **`pub(crate)`** för interna typer, inte `pub`

### Godkända undantag (kräver inte godkännande)

Dessa lints är undantagna på crate-nivå i `lib.rs` eftersom de inte är relevanta
eller praktiska för ett synthesizer-projekt:

```rust
// === Crate-level allows (i lib.rs) ===

// Namngivning och struktur
#![allow(clippy::module_name_repetitions)]  // FilterMode är tydligare än filter::Mode

// Casts - normalt i audio/DSP-kod
#![allow(clippy::cast_precision_loss)]      // usize/u64 -> f32 är standard i audio
#![allow(clippy::cast_possible_truncation)] // f32 -> usize med bounds check är OK
#![allow(clippy::cast_sign_loss)]           // Där värdet garanterat är positivt
#![allow(clippy::cast_lossless)]            // as-casts är tydligare än From i DSP

// Matematik och DSP
#![allow(clippy::suboptimal_flops)]         // mul_add() är inte alltid snabbare/tydligare
#![allow(clippy::many_single_char_names)]   // t, x, y, a, b, c är standard i matematik

// Dokumentation (intern kod, inte publikt API)
#![allow(clippy::doc_markdown)]             // Backticks i docs är pedantiskt
#![allow(clippy::missing_errors_doc)]       // Errors-sektion krävs inte internt
#![allow(clippy::missing_panics_doc)]       // Panics-sektion krävs inte internt

// Kodstil
#![allow(clippy::uninlined_format_args)]    // format!("{}", x) är OK
#![allow(clippy::redundant_closure_for_method_calls)] // .map(|x| x.foo()) är tydligare ibland

// === Lokala allows (på funktion/struct-nivå) ===

#[allow(clippy::too_many_lines)]           // Endast på process() funktioner
#[allow(clippy::similar_names)]            // left/right, peak_l/peak_r, pm/pwm
#[allow(clippy::float_cmp)]                // I tester med avsiktlig exakt jämförelse
#[allow(clippy::needless_pass_by_value)]   // Ibland för API-konsistens
```

### Vid varningar

1. Fixa varningen
2. Om omöjligt: förklara varför och vänta på godkännande
3. Om godkänd: lägg till `#[allow(...)]` med kommentar

---

## Realtidssäkerhet (audio thread)

I `process()` funktioner och annan realtidskritisk kod:

### Förbjudet
- **Heap-allokeringar:** `Vec::push`, `HashMap::insert`, `String::clone`, `Box::new`
- **Lås som kan blocka:** `Mutex::lock`, `RwLock::write`
- **Panic:** `unwrap()`, `expect()`, `panic!`, out-of-bounds indexering

### Tillåtet
- `unwrap_or(0.0)` för säkra defaults på samples
- Pre-allokerade buffers
- Atomic operationer
- Lock-free strukturer

### For-loopar vs iteratorer

**Behåll for-loopar** i audio DSP för sample-processing:
```rust
// BRA - tydligt och optimalt för audio
for i in 0..samples {
    output[i] = input[i] * gain;
}
```

**Använd iteratorer** utanför hot path:
```rust
// BRA - för collection-operationer
self.instruments.iter_mut()
    .for_each(|inst| inst.panic());
```

---

## Rust best practices

### Obligatoriska attribut

```rust
// På alla typer som returnerar värden som inte bör ignoreras
#[must_use]
pub fn invert(self) -> Self { ... }

// På alla newtypes
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct Hertz(f32);

// På builder-metoder
#[must_use]
pub fn with_frequency(self, freq: Hertz) -> Self { ... }
```

### Visibility

```rust
// Använd pub(crate) för interna typer
pub(crate) struct GraphNode { ... }

// Använd pub(super) för modul-interna helpers  
pub(super) fn helper_function() { ... }

// Endast pub för det publika API:et
pub struct Oscillator { ... }
```

### Error handling

```rust
// Använd thiserror för alla error-typer
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Invalid value: {0}")]
    InvalidValue(String),
}

// Returnera Result, inte panic
pub fn parse(input: &str) -> Result<Value, MyError> { ... }
```

### Konstruktorer och Default

```rust
// Alla typer med new() ska också ha Default
impl Default for Oscillator {
    fn default() -> Self {
        Self::new()
    }
}

// Använd const fn där möjligt
impl Hertz {
    #[must_use]
    pub const fn new(hz: f32) -> Self {
        Self(hz)
    }
}
```

### Dokumentation

```rust
/// En frekvens i Hertz.
///
/// Används för oscillatorfrekvenser, filterfrekvenser och LFO-hastigheter.
///
/// # Example
///
/// ```
/// let a4 = Hertz::A4;
/// assert_eq!(a4.as_f32(), 440.0);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Hertz(f32);
```

### Undvik

```rust
// UNDVIK: Manuell impl Display + Error
impl std::fmt::Display for MyError { ... }
impl std::error::Error for MyError {}

// ANVÄND: thiserror istället
#[derive(Debug, thiserror::Error)]
pub enum MyError { ... }

// UNDVIK: .clone() i onödan
let x = some_vec.clone();

// ANVÄND: referens eller Cow
let x = &some_vec;
let x: Cow<str> = if owned { s.into() } else { s.as_str().into() };

// UNDVIK: String som parameter
fn process(name: String) { ... }

// ANVÄND: impl Into eller &str
fn process(name: impl Into<String>) { ... }
fn process(name: &str) { ... }
```

### Iteratorer (utanför audio thread)

```rust
// ANVÄND: Metodkedja för collection-operationer
let sum: f32 = values.iter().filter(|x| x.is_valid()).sum();

// ANVÄND: for_each för side effects
self.modules.values_mut().for_each(|m| m.reset());

// ANVÄND: find_map för sökning
let found = items.iter().find_map(|item| item.get_value());
```

### Konstanter

```rust
impl Hertz {
    // Använd associated constants
    pub const A4: Self = Self(440.0);
    pub const MIN_AUDIBLE: Self = Self(20.0);
    pub const MAX_AUDIBLE: Self = Self(20_000.0);
}
```

---

## Efter varje svar

Skriv ut `/usage` så jag ser hur mycket som är kvar.