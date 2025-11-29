# TODO - Visualiseringar & Analysverktyg

Målet är att ge användaren visuell feedback på ljudet i realtid. Eftersom vi redan har `VisualizationBuffer` med rådata kan vi bygga dessa i GUI-tråden utan att störa ljudmotorn.

## 🟢 Prioritet 1: Grundläggande Analys (Mest användbart)

Dessa verktyg är standard i musikproduktion och bör finnas i en "seriös" synth.

### 1. Spektrumanalysator (FFT)
Visar frekvensinnehållet i ljudet.
- **Beskrivning:** En graf där X-axeln är frekvens (logaritmisk 20Hz-20kHz) och Y-axeln är amplitud (dB).
- **Utseende:** Fyllda staplar eller en mjuk linjekurva. Gärna med "peak hold" (en linje som stannar kvar lite längre på högsta nivån).
- **Implementation:** - Använd craten `rustfft` för att transformera tidsdomän (samples) till frekvensdomän.
    - Applicera en fönsterfunktion (t.ex. Hann window) för att minska "spectral leakage".
    - Rita med `egui::Painter`.

### 2. Vectorscope (Goniometer)
Visar stereobredden och fasrelationen mellan höger och vänster kanal.
- **Beskrivning:** Ett "trasselsudd" som rör sig i en diamantform eller cirkel.
    - Vertikal linje = Mono.
    - Horisontell linje = Ur fas (dåligt för mono-kompatibilitet).
- **Implementation:** - Rotera L/R koordinaterna 45 grader: `X = L - R`, `Y = L + R`.
    - Rita punkter eller linjer med snabb "fade out" för att skapa spår.

### 3. Fas-korrelationsmätare
Ett enkelt komplement till Vectorscope.
- **Beskrivning:** En enkel liggande stapel från -1 (rött, ur fas) till +1 (grönt, i fas).
- **Implementation:** Beräkna korrelationen mellan L och R samples över en buffert.

---

## 🟡 Prioritet 2: "Cool-faktor" & Arbetsflöde

Dessa gör synthen roligare att använda och hjälper vid ljuddesign.

### 4. Spektrogram (Vattenfall / Waterfall)
Som en spektrumanalysator men med historik.
- **Beskrivning:** Frekvenser rullar "nedåt" (eller åt sidan) över tid. Färg indikerar styrka (Svart -> Blå -> Röd -> Vit).
- **Cool-faktor:** Hög! Ser väldigt proffsigt ut och man kan se hur övertoner utvecklas över tid.
- **Implementation:** Kräver en textur/bildbuffert som uppdateras rad för rad.

### 5. Kromatisk Stämapparat (Tuner)
Hjälper till att stämma oscillatorer, särskilt vid FM-syntes.
- **Beskrivning:** Visar närmaste not (t.ex. "C#4") och en nål/visare för finstämning (cents).
- **Implementation:** Enklast är en "Zero Crossing"-algoritm (räkna hur ofta vågen korsar nollinjen) kombinerat med ett lågpassfilter för att få bort övertoner. För mer precision: Autokorrelation.

### 6. Röst-aktivitet (Polyfoni-LEDs)
Visar vad motorn gör "under huven".
- **Beskrivning:** En rad med 8 (eller max polyfoni) små "lampor".
- **Funktion:** - Lampan tänds när en röst används.
    - Färgen kan indikera Velocity (anslagsstyrka).
    - En "R"-indikator om rösten är i Release-fasen.
- **Varför:** Hjälper användaren förstå röst-stöld (voice stealing) och om polyfonin räcker till.

---

## 🔵 Prioritet 3: Modulations-visualisering (Slow Scope)

Eftersom detta är en modulär synth är det kritiskt att se *styrsignaler* (CV), inte bara ljud.

### 7. CV-Oscilloskop
För att se LFO:er och Envelopes.
- **Beskrivning:** Ett oscilloskop optimerat för långsamma signaler.
- **Skillnad mot audio-scope:** - Mycket långsammare tidsskala (flera sekunder per skärm).
    - Ingen DC-offset filtrering (vi vill se om en LFO går från 0 till 1 eller -1 till 1).
    - "Triggad" ritning: Börja rita när en Gate-signal kommer (för att se exakt hur en Envelope ser ut).

### 8. Modulations-trådar (Live Cables)
- **Idé:** Kablarna mellan moduler pulserar eller ändrar tjocklek/färg baserat på signalstyrkan som går genom dem.
- **Implementation:** Kräver att vi skickar viss signaldata (t.ex. RMS-värde) från ljudtråden för varje koppling. Kan vara prestandakrävande men ser väldigt levande ut.

---

## 🟣 Experimentellt / Avancerat

### 9. 3D Waveform Terrain
- **Beskrivning:** Som Joy Divisions "Unknown Pleasures"-omslag. Flera vågformer staplas bakom varandra i 3D-perspektiv.
- **Implementation:** Kräver lite mer avancerad `egui`-målning eller integration med `three-d` eller liknande om man vill ha riktig 3D, men kan fejkas med 2D-linjer.

### 10. Lissajous-figurer (X/Y Input)
- **Beskrivning:** Låt användaren koppla valfri signal till X och valfri till Y.
- **Användning:** Skapa komplexa geometriska mönster genom att koppla olika oscillatorer/LFO:er mot varandra.