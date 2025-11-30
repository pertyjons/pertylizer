# GUI-Engine Communication Specification (v0.23.0)

Detta dokument beskriver hur det grafiska gränssnittet (GUI) kommunicerar med ljudmotorn (`SynthEngine`) i Modular Synth. Systemet är designat för att vara **lock-free** och **thread-safe** för att garantera att ljudtråden aldrig blockeras av GUI-operationer.

## Översikt

Kommunikationen sker via tre primära kanaler:

1.  **Command Queue (GUI -> Engine):** En `ringbuf` där GUI skickar kommandon (`EngineCommand`) till ljudmotorn.
2.  **Event Queue (Engine -> GUI):** En `ringbuf` där ljudmotorn skickar statusuppdateringar (`EngineEvent`) till GUI.
3.  **Shared State (Atomic):** Delad data (`Arc<EngineState>`) för "billiga" läsningar som mätare och transport-position.
4.  **Visualization Buffers (Shared Memory):** Delade `Arc<VisualizationBuffer>` för att överföra stora mängder ljuddata (oscilloskop/spektrum) utan kopiering.
5.  **Return Queue (Engine -> GUI):** En kanal för att skicka tillbaka objekt (t.ex. borttagna moduler) som ska avallokeras på huvudtråden för att undvika `drop()` i ljudtråden.

---

## 1. Kommandon (GUI -> Engine)

Definierade i `src/engine/commands.rs`. Dessa kommandon skickas asynkront och hanteras i början av varje ljud-block.

### Not-hantering
* `NoteOn { note: u8, velocity: f32, channel: u8 }` - Starta en not (kanal mappas till `PartId`).
* `NoteOff { note: u8, channel: u8 }` - Stoppa en not.
* `AllNotesOff` - Stäng av alla noter (panic).

### Parameter-styrning
Parametrar använder nu det typsäkra systemet definierat i `src/engine/typed_params.rs`.

* `SetVoiceParameter { target: VoiceModule, param: TypedParam, value: TypedValue }`
    * Styr parametrar i röst-kedjan (t.ex. Oscillator, Filter).
    * `target`: Enum som pekar ut modul (t.ex. `VoiceModule::Oscillator1`, `VoiceModule::Filter`).
* `SetEffectParameter { effect_type: EffectType, param: TypedParam, value: TypedValue }`
    * Styr parametrar i effektkedjan (t.ex. Delay, Reverb).
* `SetModuleParameter { module_id: ModuleId, param: TypedParam, value: TypedValue }`
    * Styr parametrar i den generella modulgrafen (för patchade moduler).

### Modul-hantering & Routing
Dessa kommandon hanterar den dynamiska strukturen. Moduler skapas i GUI-tråden och skickas till motorn.

* `AddModuleInstance { id: ModuleId, module: Box<dyn VoiceModule> }` - Lägg till en ny modul i grafen.
* `RemoveModule { id: ModuleId }` - Ta bort en modul.
* `Connect { from: PortId, to: PortId }` - Koppla ihop två portar.
* `Disconnect { from: PortId, to: PortId }` - Ta bort en koppling.
* `DisconnectAll { module: ModuleId }` - Ta bort alla kopplingar till/från en modul.

### Multitimbralitet & Parts
* `AddPart { id: PartId, part: Box<SynthPart> }` - Lägg till ett nytt instrument.
* `RemovePart { id: PartId }` - Ta bort ett instrument.

### Effekter & Visualisering
* `AddEffectInstance { id: ModuleId, effect: Box<dyn EffectModule> }` - Lägg till en effekt i master-kedjan.
* `RemoveEffect { id: ModuleId }` - Ta bort en effekt.
* `SetEffectEnabled { effect_type: EffectType, enabled: bool }` - Bypass på effekt.
* `AddVisualizer { id: ModuleId, visualizer_type: VisualizerType, buffer: Arc<VisualizationBuffer> }`
    * Kopplar en visualisator. Notera att `buffer` delas mellan trådarna.
* `RemoveVisualizer { id: ModuleId }` - Tar bort visualisatorn.

### Globala Inställningar
* `SetMasterVolume(f32)`
* `SetGlideTime(f32)` - Sätter portamento-tid i sekunder.
* `SetTempo(f32)` - Sätter BPM.
* `ClearAllModules` - Rensar allt (vid patch-laddning).

---

## 2. Events (Engine -> GUI)

Definierade i `src/engine/commands.rs`. Skickas från ljudtråden för att uppdatera gränssnittet.

* `PeakMeter { left: f32, right: f32 }` - Momentan ljudnivå (för VU-metrar).
* `RmsMeter { left: f32, right: f32 }` - Genomsnittlig ljudnivå.
* `VoiceCount(u32)` - Antal aktiva röster just nu.
* `CpuUsage(f32)` - Mätt processortid för ljud-callbacken (0.0 - 1.0).
* `BufferUnderrun` - Varning om ljudbufferten inte hann fyllas i tid.
* `ParameterChanged { module: ModuleId, param: TypedParam, value: TypedValue }` - Eko på parameterändring (för automation/Macro).
* `EnvelopeStage { module: ModuleId, stage: u8 }` - Visuell feedback på envelope-fas (Attack, Decay, etc).

---

## 3. Typsystem (`TypedParam` & `TypedValue`)

För att garantera typsäkerhet över trådgränserna används inga råa `f32` eller strängar för parametrar.

**TypedValue** (Enum):
* `Float(f32)`
* `Int(i32)`
* `Bool(bool)`
* `Waveform(Waveform)` - Enum för oscillator-vågformer.
* `FilterMode(FilterMode)` - Enum för filtertyper (LP, HP, BP...).
* `MusicalTime(MusicalTime)` - För taktsynkade värden.

**TypedParam** (Nested Enum):
Kapslar in både modul-typ och parameter-ID för att förhindra felaktig adressering.
* Exempel: `TypedParam::Oscillator(OscillatorParam::Waveform)`
* Exempel: `TypedParam::Filter(FilterParam::Cutoff)`

---

## 4. Visualisering (Shared Memory)

För oscilloskop och spektrumanalys är Events för långsamma och bandbreddskrävande.

* **Mekanism:** `Arc<VisualizationBuffer>` (definierad i `src/visualizers/mod.rs`).
* **Flöde:**
    1.  GUI skapar en `VisualizationBuffer` (innehåller atomics och ringbuffertar).
    2.  GUI skickar bufferten till Engine via `AddVisualizer`-kommandot.
    3.  Engine skriver råa samplingar till ringbufferten i realtid (lock-free write).
    4.  GUI läser från ringbufferten vid rendering (lock-free read).
* **Fördel:** Ingen kopiering av data, ingen blockering av ljudtråden, hög uppdateringsfrekvens (60fps+).

---

## 5. Minneshantering (Return Queue)

För att uppfylla realtidskrav får minne inte avallokeras (`drop`) i ljudtråden.

* När en modul tas bort via `RemoveModule`:
    1.  Engine tar bort modulen ur grafen/listan.
    2.  Istället för att droppa den, skickas den tillbaka till GUI via `return_producer`.
    3.  GUI-tråden (i `update`-loopen) pollar kön och droppar objekten säkert.

```rust
pub struct DroppedModule(pub Box<dyn VoiceModuleTrait>);