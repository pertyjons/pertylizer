# Plan för Uppgradering av Visualizer (Bevy 0.18 & Shaders)

Denna plan är baserad på en djupgående systematisk granskning av `visualizer/`-koden. Projektet har en utmärkt struktur (med effect layers, crossfades, teman och SystemSets) och koden är till stor del anpassad till Bevy 0.15+, men det finns vissa ECS-mönster som kan göras mer idiomatiska för Bevy 0.18. 

Ännu viktigare: Genom att flytta logik från CPU (massiva uppdateringar av `Transform`- och `StandardMaterial`-komponenter) till **GPU via custom shaders** kan applikationen gå från en "enkel Bevy-app" till ett högpresterande, professionellt VJ-verktyg.

---

## 1. Bevy 0.18 & Idiomatiska ECS-Konventioner

### 1.1 Byt ut manuella Vec-buffrar mot Bevys inbyggda `Event`-system
**Problem:** Transienta händelser som `pending_note_events` buffras i `SynthTelemetry`-resursen och rensas manuellt (`clear()`) i varje frame. Detta är en känd "anti-pattern" i ECS.
**Lösning:** Använd Bevys kraftfulla event-system.
*   Registrera events i `main.rs`: `app.add_event::<NoteOnEvent>();` och `app.add_event::<CameraModeRequest>();`.
*   I `osc_receiver.rs` (`receive_osc`), skicka events via `EventWriter<NoteOnEvent>`.
*   I effekter (t.ex. `particles::spawn`, `instrument_cubes::spawn`), lyssna med `EventReader<NoteOnEvent>`. Bevy hanterar automatiskt buffring och radering efter att alla system läst klart.

### 1.2 Dra nytta av "Required Components"
**Möjlighet:** Från Bevy 0.15+ är komponentsystemet smartare. `Mesh3d` "kräver" `Transform` och `Visibility`.
**Lösning:** I dina `setup()`-funktioner behöver du egentligen inte manuellt lägga till `Transform::default()` eller `Visibility::Hidden` om du bara vill ha default-värdena. Att du explicit sätter `Visibility::Hidden` direkt är dock bra för crossfade-logiken, men koden kan göras renare genom att bara spawna `(Mesh3d, MeshMaterial3d)` där anpassad Transform inte behövs direkt vid spawn.

### 1.3 Undvik att mutera `StandardMaterial` frekvent vid Fades
**Problem:** I t.ex. `effects::update_hue_materials_for_fade` modifieras egenskaperna på delade material i varje frame under en övergång (`fade`). Även med "material bucketing" (som är snyggt implementerat) triggas en onödig uppladdning till GPU:n.
**Lösning (Långsiktig):** För enklare effekter gör du redan rätt (använder `Transform.scale` för att fejka fade). För färg och emissive-styrka, överväg att hantera master-fade via post-processing eller genom att injicera en global uniform (via custom shaders) istället för att mutera CPU-sidans material i en loop.

---

## 2. Den Stora Övergången till Shaders (Prestanda & Visuellt Lyft)

Genom att använda GPU:n för tunga uppdateringar kan du frigöra CPU:n från att uppdatera tusentals `Transform`-komponenter. Här är de 5 starkaste kandidaterna:

### 2.1 `spectral_waterfall.rs` (Vertex Displacement Shader)
**Nu:** Spawnar 2048 (64x32) unika kuber och muterar `Transform` i Z- och Y-led varje frame. Mycket CPU-arbete.
**Shader-lösning:** 
*   Använd **ett enda** `Plane3d`-mesh med 64x32 subdivisions.
*   Skicka in FFT-historiken som en rullande 2D-textur eller `StorageBuffer` till en Vertex Shader.
*   Shadern förskjuter vertex-punkternas höjd (`Y`) baserat på FFT-värdet, och sätter färg i Fragment Shadern baserat på X-koordinat/höjd.
*   *Resultat:* 1 Entitet, 1 Draw call. Sömlös, silkeslen scrolling utan CPU-overhead.

### 2.2 `reaction_diffusion.rs` (Compute Shaders)
**Nu:** Kör Gray-Scott simulering på en 12x12 grid på CPU:n i 15 Hz.
**Shader-lösning:** 
*   Flytta simuleringen till en **Compute Shader** och skala upp gridden till 256x256 eller 512x512 pixlar som uppdateras i 60 Hz.
*   Använd `telemetry.centroid_hz` och `rms` som uniforms för att styra reaktionshastigheten (feed/kill).
*   Använd den genererade texturen antingen direkt på en yta eller för att driva displacement av golvet, vilket skapar levande, organiska fraktalmönster.

### 2.3 `ferrofluid_tendrils.rs` (Raymarching / SDF)
**Nu:** Vertikala cylindrar som ändrar höjd och tiltar. Ser inte ut som en kohesiv vätska.
**Shader-lösning:** 
*   Implementera **Signed Distance Fields (SDF)** och **Raymarching** i en Fragment Shader på en enkel kub-volym.
*   Rita sfärer från botten som drivs av FFT-värden. Genom att använda `smin` (smooth minimum) i shader-matematiken "smälter" pelarna samman precis som riktig ferrofluid eller kvicksilver.
*   Gör effekten extremt fotorealistisk och fängslande.

### 2.4 Terräng och Deformation (`pulse_terrain`, `fft_terrain`, `voronoi_shatter`)
**Nu:** Loopar hundratals entiteter på CPU:n för att applicera sinusvågor eller höjddata via `Transform`.
**Shader-lösning:** 
*   Skriv en **Custom Material Extension** (`impl MaterialExtension for MyTerrain`).
*   Sköt all våg-matematik (ripple, t*5.0) direkt i Vertex Shadern.
*   Tillåter att öka griddens storlek dramatiskt (ex. 200x200) för att få massiva landskap utan att tappa ens 1 FPS.

### 2.5 GPU-Partiklar (`particles`, `velocity_meteors`, `instrument_cubes`)
**Nu:** Hårda gränser på max 512 entiteter för att det inte ska lagga.
**Shader-lösning:** 
*   Använd ett externt bibliotek som **`bevy_hanabi`** (GPU-partikelsystem) eller skriv en enkel Compute Shader för GPU Instancing.
*   Tillåter 100 000+ partiklar.
*   Spawna partiklar via Bevy Events från din OSC-tråd och låt GPU:n hantera gravitation, krympning och kollisioner.

---

## 3. Visuell Polering av Befintliga Effekter (Färg, Djup & Variation)

Många effekter använder just nu ett enda delat material för hundratals instanser, vilket skapar en "monokrom" eller platt känsla. Genom att introducera material-buckets (som i `fft_bars`) och variera färg, saturation och alpha beroende på position kan vi få fram enormt mycket mer djup.

### Akuta Förbättringar (För lite färg/djup)
- [x] **`centroid_nebula` (500 sfärer):** Byt ut det ensamma materialet mot ~16 buckets via `create_hue_materials`. Välj material baserat på position `(x+y+z) % 16` och variera lightness/saturation för en mer levande nebulosa.
- [x] **`note_tree` (L-system):** Skapa olika material per förgreningsnivå (`level`). Mörkare/lägre emissive på stammen, ljusare/mer emissive på yttre grenarna. Eventuellt lägga till färgade "löv" vid note-events.
- [x] **`ferrofluid_tendrils`:** Istället för ett material, mappa `band_index` mot `band_frequency_hue()`. Röd/orange bas i mitten och blå/lila toppar i utkanterna gör vågrörelserna extremt mycket tydligare.
- [x] **`pulse_terrain`, `spectral_origami` & `voronoi_shatter` (Grids):**
  - [x] *Pulse Terrain:* Mappa lightness/färg mot `dist_from_center` (ljust/varmt i mitten, mörkt/kallt i kanterna).
  - [x] *Spectral Origami:* Mappa färg mot `index % buckets` för variation, låt eventuellt baksidan bryta av i färg.
  - [x] *Voronoi Shatter:* Variera saturation per bit för att förtydliga att de är separata skärvor.

### Ytterligare Polering (Dynamik)
- [x] **`chord_bloom`:** Välj olika färg-buckets för olika kronblad (segment) i blomman, t.ex. varannan bucket +1, eller gör kärnan ljusare med högre emissive.
- [x] **`harmonic_ribbons`:** Låt Z-skalan pulsera mer drastiskt med `telemetry.pitch_bend`, och på sikt (i shaders) fadea färgen på svansen mot mörkblått/kallt.
- [x] **`instrument_cubes` & `velocity_meteors`:** Öka rotationen avsevärt baserat på `flux`/`rms`. Lägg till en liten chans (~5%) för en "crit" (dubbelt så stor, vit emissive).
- [x] **`fft_terrain`:** Fadea `lightness` bakåt i Z-led. Pelarna närmast kameran ska vara skarpast, medan de längst bak smälter in i mörkret.

---

## 4. Nya Effektidéer (Kreativ Expansion)

- [x] **`cyber_wireframe` (Terrain / Base):** Trådmodeller av tunna cylindrar i grid. Noderna hoppar med FFT, linjerna pulserar med beat_phase, färg sveper som en radar med beat_position.
- [x] **`orbital_satellites` (Hero):** En ring av "satelliter" som svävar runt centrum. Vid `last_note_on` skjuter de tunna lasrar in mot mitten och rekylerar. Rotationen styrs av tempo/flux.
- [x] **`spectral_aurora` (Ambient / Sky):** Mjuka vågande draperier högt uppe (`y=20.0`) likt norrsken. Amplituden styrs av `rms` och färgskiftningar triggas av `centroid_hz`. Ger en mjuk, rökig kontrast till andra hårda geometrier.
- [x] **`beat_fracture` (Transients / Actions):** Hela luften "spricker" som glas vid extrem `event_drops` eller `flux`+`rms`. Slumpmässiga, nästan genomskinliga men neonkantade skärvor som hänger i luften i 0.2s och sen faller.

---

## 5. Prioriterad Handlingsplan (Roadmap)

- [x] **Steg 1: Förenkla arkitekturen (Quick Win)**
  Refaktorera OSC -> Telemetry till att använda `EventWriter` / `EventReader` för not-händelser och kamerabyten istället för att manuellt tömma listor.
- [x] **Steg 2: Material-buckets i CPU-effekter (Quick Win)**
  Uppdatera de befintliga effekterna enligt avsnitt 3, vilket drastiskt minskar den platta "monokroma" känslan med befintlig CPU-kod.
- [x] **Steg 3: Introducera Custom Materials (Shaders)**
  Konvertera `spectral_waterfall` till en enda mesh och en egen Vertex Displacement Shader. (Läs in dig på Bevys `shader_material.rs` och Asynchronous Asset Loading för texturer/buffrar).
- [x] **Steg 4: Applicera shader-tekniken på all terräng**
  Gör samma sak med `pulse_terrain` och `fft_terrain` när du blivit bekväm med Material Extensions i Bevy.
- [x] **Steg 5: Experimentera med Compute Shaders**
  Titta på Bevys officiella exempel för Game of Life (`compute_shader_game_of_life.rs`) och applicera det på `reaction_diffusion`.
- [x] **Steg 6: Extrema visuella effekter (Shaders/Particles)**
  Undersök Raymarching i Fragment Shaders för `ferrofluid_tendrils` och titta på integration av `bevy_hanabi` för att ta partikelsystemen (`particles`, `velocity_meteors`) till nästa nivå.
