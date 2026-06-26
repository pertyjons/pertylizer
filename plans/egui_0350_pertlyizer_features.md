# Egui 0.35.0 Nyheter & Pertylizer-integration

Här är en utvärdering och sammanfattning av de mest spännande nyheterna i **egui 0.35.0** och hur vi kan använda dem för att lyfta Pertylizer till en ny nivå av kodkvalitet, UX och testbarhet.

---

## 1. Egui Inspection & `egui_mcp` (Milstolpe för MCP!)

Detta är förmodligen den största nyheten för oss, då Pertylizer redan har en djup MCP-integration.

* **Vad det faktiskt är (verifierat 2026-06-25).** Projektet heter
  [`rerun-io/kittest_inspector`](https://github.com/rerun-io/kittest_inspector)
  och består av två delar: protokoll-cratet **`egui_inspection`** (som eframe drar
  in via sin **`inspection`**-feature, med `features = ["plugin"]` så *eframe
  själv* registrerar inspektions-pluginet — ingen manuell `enable_accesskit()`
  eller server-spawn behövs) och MCP-servern **`egui_mcp`** (binären `egui-mcp`).
  Vår `eframe 0.35.0` har `inspection`-featuren inbyggd
  (`inspection = ["dep:egui_inspection", "accesskit"]`), så integrationen är
  nästan noll kod.
  > **OBS — INTE att förväxla** med tredjeparts-cratet `dijdzv/egui-mcp`
  > (`egui-mcp-server` 0.0.5), som är ett helt separat, omoget projekt byggt på
  > Linux AT-SPI + manuell `IpcServer`-spawn. Vi använder rerun-io-varianten.
* **Så slår vi på det.** Lägg en **opt-in** cargo-feature (ej i `default`, så
  release/CI-matrisen inte får med `accesskit` i onödan):
  ```toml
  # crates/pertylizer/Cargo.toml
  egui-inspection = ["eframe/inspection"]
  ```
  Kör sedan med miljövariabeln satt (osatt/`0`/`false` = helt avstängt):
  ```bash
  EGUI_INSPECTION=1 cargo run --features egui-inspection
  # pluginet binder 127.0.0.1:5719
  ```
  Installera och registrera servern:
  ```bash
  cargo install --git https://github.com/rerun-io/kittest_inspector egui_mcp
  claude mcp add egui egui-mcp        # eller "egui": { "command": "egui-mcp" } i ~/.claude.json
  ```
  Verktyg: `query_tree`/`get_node` (AccessKit-trädet), klick/typ/scroll/drag/
  tangenttryck, screenshots, fönsterresize, async-vänta-på-UI. Cross-platform
  (screenshot-varning bara på macOS för ockluderade fönster).
* **Hur Pertylizer drar nytta av det:**
  När du ber mig (eller en annan AI-agent) att testa GUI:t kan jag driva
  `egui-mcp`: "se" komponentträdet, klicka knappar, dra reglage och verifiera att
  gränssnittet fungerar i realtid — autonom in-app-verifiering, vilket stänger vår
  återkommande "byggt grönt men aldrig eyeballat"-lucka.
* **⚠ Begränsning som måste hanteras — AccessKit-täckning.** Servern ser bara det
  som finns i **AccessKit-trädet**, dvs widgets som sätter `WidgetInfo`. Standard-
  widgets (knappar, sliders, combos, labels, textfält) gör det automatiskt, men
  våra **painter-ritade canvas-widgets** (`knob.rs`, `port.rs`, `cable.rs`, och
  kommande Scene-noder) sätter **ingen** `WidgetInfo` (verifierat: noll
  `widget_info`/`accesskit`-träffar i `widgets/`). Konsekvens: agenten kan driva
  standard-UI men ser **inte** knobar/portar/kablar/noder som strukturerade noder
  — bara blind `click_at`/`drag` på skärmkoordinater. För att kunna verifiera
  **patch-editorns nod-/wire-dragning trädbaserat** måste vi lägga
  `response.widget_info(...)` på dessa widgets. Det hör hemma i **Phase 2**
  (`ModuleNode`/`node.rs`) i patch-editor-omskrivningen — bygg accesskit-noderna
  samtidigt som vi ändå rör de widgetarna. (Bra för tillgänglighet oavsett MCP.)
* **Spike-resultat (2026-06-25) — verifierat end-to-end mot levande app.**
  Cargo-featuren `egui-inspection` lades till (opt-in, ej default) i
  `crates/pertylizer/Cargo.toml`; bygget är grönt mot 0.35 och `egui_inspection`
  länkas *bara* med featuren (default-bygget länkar inte accesskit).
  `EGUI_INSPECTION=1 ./target/debug/pertylizer` binder `127.0.0.1:5719` inom ~2 s
  utan en rad appkod. `egui-mcp`-servern installerad
  (`cargo install --git … egui_mcp`), `attach` + `query_tree` mot tomt projekt gav
  **88 noder**: 21 `Button` (alla märkta), 12 `Label`, 1 `Slider`, 1 `SpinButton`,
  1 `Image` — och **7 `Unknown`-noder**: de 6 vy-väljarflikarna i toppraden
  (Patch/Mixer/Sequencer/…) och hela keyboard-panelen (en enda 1200×100-blob).
  Dvs custom-ritade widgets dyker upp **positionerade men utan roll/label** — exakt
  som förutsagt. **Konkret omedelbar vinst:** vy-flikarna saknar label, så en agent
  kan i dag inte ens *navigera till patch-editorn via namn* — ge dem
  `widget_info` (billigt, gör det före/utöver Phase 2). Tom canvas hade 0 moduler
  att mäta; modul-täckning bekräftas dock redan av att flikarna/keyboarden faller
  ut som `Unknown`. Drivare: `scratchpad/mcp_probe.py` + `mcp_stats.py`.

---

## 2. CSS-liknande klasser (`UiBuilder::with_class`)

Egui 0.35 introducerar stöd för CSS-liknande klasser på `Ui`-containrar.

* **Vad är det?** Du kan sätta en klass på en `Ui` och sedan kontrollera längre ner i komponentträdet om den är kapslad inuti en sådan klass:
  ```rust
  // Skapa en container med klassen "compact"
  ui.scope_builder(UiBuilder::new().with_class("compact"), |ui| {
      // Rita widgets...
  });

  // Inuti en anpassad widget eller reglage:
  let is_compact = ui.stack().iter().any(|s| s.classes.has("compact"));
  if is_compact {
      // Anpassa storlek och layout automatiskt!
  }
  ```
* **Hur Pertylizer drar nytta av det:**
  I vår omskrivningsplan för patch-editorn kan vi använda detta för att hantera **layout-konditioner**. Istället för att skicka runt booleans som `compact: bool` till alla parametrar och knoppar, kan modulen sätta en klass (t.ex. `"compact-rack"` eller `"mod-matrix-zone"`). Alla underliggande widgets i `param_grid.rs` kan sedan anpassa sin storlek och textstorlek dynamiskt utifrån detta sammanhang.

---

## 3. Förbättrade animationer & panel-UX

Animationer och paneler har fått en stor visuell uppgradering som ger en mycket mer "premium" känsla.

* **Sliding Panels:** Paneler har nu mjuka "slide"-animationer när de öppnas och stängs, och animationstiden har saktats ner från 0.1s till 0.2s för en mer naturlig rörelse.
* **Double-click Resize Toggle:** Användare kan nu dubbelklicka på kanten av en panel för att snabbt expandera/kollapsa eller nollställa storleken (precis som i professionella DAW-program).
* **Drag-to-close:** Det finns inbyggt stöd för att dra paneler för att stänga dem.
* **Hur Pertylizer drar nytta av det:**
  Detta förbättrar direkt användarupplevelsen i våra sidopaneler (t.ex. instrumentlistan) och det flyttbara keyboard-gränssnittet. Animationerna känns mjukare och möter våra premium UX-krav ("WOW-faktor").

---

## 4. Atoms i Window-titlar och Small Buttons

Vi använder redan eguis *Atoms* (sammansatta ikoner/textgrupper) för parameteretiketter i `param_grid.rs`.

* **Vad är det?** Egui 0.35 lägger till atom-stöd i fönstertitlar (`egui::Window`) och små knappar (`Ui::small_button`).
* **Hur Pertylizer drar nytta av det:**
  När vi renderar flytande popup-menyer och editörer (t.ex. skripteditorn eller beskrivningseditorn) kan vi nu använda atoms direkt i fönstrets titlebar för att enkelt visa ikoner (från `egui_remixicon`) och text med perfekt baslinjejustering utan manuellt fuskande.

---

## 5. OS-nivå Custom Cursors (`Context::set_cursor_image`)

* **Vad är det?** Egui stöder nu att sätta anpassade markörbilder (`set_cursor_image`) på OS-nivå, snarare än att bara förlita sig på standard systemmarkörer.
* **Hur Pertylizer drar nytta av det:**
  Under kabeldragning (`CanvasInteraction::DraggingWire`) kan vi rita en anpassad tråd-ikon eller en specialdesignad markör som visar en liten kontakt, vilket förstärker den fysiska känslan i synth-editorn.

---

## 6. Text- & Typografiförbättringar (`harfrust` & subpixel binning)

* **Harfbuzz-ligaturer:** Egui använder nu `harfrust` (en rust-port av Harfbuzz) för textformatering. Detta ger markant bättre kerning och stöd för ligaturer.
* **Subpixel Binning:** Nya alternativ i `TextOptions` ger skarpare rendering av text, särskilt på skärmar utan hög DPI.
* **Hur Pertylizer drar nytta av det:**
  Text och teckensnitt (som Outfit eller Roboto) kommer att se mycket mer professionella och "krispiga" ut, vilket förhindrar att synth-gränssnittet känns suddigt vid udda zoomnivåer i patch-editorn.
