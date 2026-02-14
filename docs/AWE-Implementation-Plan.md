# Implementation Plan: The Acoustic World Engine (AWE)

## 1. Vision & Core Concept

The Acoustic World Engine (AWE) is envisioned as a final processing stage that gives synthesized sounds a physical **space** to exist in. It moves beyond standard convolution reverb by creating a *dynamic, interactive acoustic environment* where sound waves reflect off surfaces, interact with virtual objects, and respond to physical laws.

This space is not limited to simple rooms. The goal is to simulate everything from a small studio to a **1km long steel pipeline, a complex cave system, or other abstract shapes**.

Unlike a standard module in the patch editor, the AWE will have its own dedicated, immersive view in the user interface, allowing for visual and intuitive manipulation of the simulated space.

**Core Architecture:**
The AWE will be built on a hybrid model:
1.  **A "World Thread":** A lower-priority thread will run a geometric acoustics simulation (ray tracing) to calculate the space's response to a sound source.
2.  **An Impulse Response (IR) Generator:** The result of the ray tracing will be a dynamically generated Impulse Response that represents the space's unique acoustic signature.
3.  **A Real-Time Convolution Engine:** On the main audio thread, the synth's output will be processed by the existing `PartitionedConvolver` effect, which will use the dynamically generated IR from the World Thread.

---

## 2. Design Decisions (Baserat på din feedback)

This plan has been updated to reflect the following key design goals:

*   **Flexible Environments:** Motorn måste kunna hantera godtyckliga 3D-former, inte bara rektangulära rum. Det kan vara allt från långa rör till komplexa, importerade meshar.
*   **Advanced Materials:** Material ska ha detaljerade fysiska egenskaper, inklusive frekvensberoende `absorption`, `diffusion` (spridning) och `hardness` (reflexivitet/hårdhet).
*   **GUI View:** En **3D-perspektivvy** som användaren kan navigera i.
*   **Interaction:** Användaren interagerar primärt via **Drag-and-Drop** av objekt.
*   **Audio I/O:** Systemet har **två ljudkällor (stereo in)** och **två lyssnare (stereo ut)**, vars positioner, höjd och avstånd kan justeras fritt i 3D-rymden.
*   **Real-time Visuals:** Simuleringen ska visualiseras i realtid, inklusive **visning av ljudstrålar (rays)**.
*   **Audio Routing:** AWE fungerar som en **Master Effect** som processar hela slutmixen.
*   **Liveness:** IR-uppdateringar sker **kontinuerligt** när objekt flyttas för omedelbar auditiv feedback.
*   **Realism:** Resonatorer (harpa, vindspel) ska kunna **interagera med varandra**, vilket skapar ett mer komplext och realistiskt system.
*   **Modulation:** Parametrar i AWE (rummets storlek, objektpositioner etc.) ska kunna **moduleras av LFO:er/envelopes** från den vanliga patch-miljön.

---

## 3. Performance Considerations & Risks

De valda designmålen beskriver ett extremt kraftfullt och dynamiskt system, men också ett som är **mycket beräkningsintensivt**. Att uppnå detta med bibehållen realtidsprestanda är den största utmaningen.

*   **Complex Geometry:** Ray tracing mot generiska meshar är betydligt mer krävande än mot enkla primitiva former som kuber. Detta ökar risken för prestandaproblem i "World Thread".
*   **Kontinuerlig IR-uppdatering:** Detta är den största risken. Att köra en full ray tracing-simulering och generera en ny IR på varje "frame" när ett objekt dras kommer att kräva enorm CPU-kraft. Initiala implementationer måste fokusera på optimering, t.ex. genom att använda ett lägre antal strålar eller färre reflektionsstudsar under förflyttning.
*   **Interagerande Resonatorer:** Kräver en feedback-loop där resonatorernas output blir en del av inputen för nästa simulerings-tick. Detta ökar komplexiteten och CPU-lasten avsevärt.
*   **Realtidsvisualisering:** Att rita ut tusentals strålar i 3D i GUI-tråden kommer att vara en flaskhals. En strategi kan vara att bara visualisera en liten delmängd av de totala strålarna.

**Strategi:** Implementationen bör ske stegvis med prestanda i åtanke. Börja med en "On Mouse Release"-uppdatering och arbeta mot en optimerad kontinuerlig uppdatering.

---

## 4. Implementation Phases

### Phase 1: The Core Engine (The Environment)

*Mål: Skapa ett flexibelt system för att definiera och simulera ett akustiskt 3D-utrymme.*

1.  **Flexible Geometry Engine:**
    *   Skapa `awe_engine` som körs i en egen tråd.
    *   Designa ett system för att definiera 3D-miljöer. Istället för bara primitiva former, fokusera på en motor som kan hantera **generiska 3D-meshar**. Detta tillåter import av komplexa former eller skapande av former via CSG (Constructive Solid Geometry).
    *   Implementera en ray tracer som är optimerad för att köras kontinuerligt mot dessa generiska meshar.

2.  **Advanced Materials & Dynamic IR Generation:**
    *   Konvertera ray tracing-resultat till ett **stereo Impulse Response**.
    *   Utöka `Material`-egenskaperna till att inkludera:
        *   `absorption`: En array med 6 värden för frekvensberoende dämpning.
        *   `diffusion`: Ett värde (0-1) som styr hur mycket en stråle sprids slumpmässigt vid reflektion.
        *   `hardness`: Ett värde som kan påverka klangen vid reflektion (t.ex. bevara höga frekvenser).
    *   Implementera justerbar ljudhastighet.

3.  **Integration med Audio Engine:**
    *   Använd en trådsäker kö (t.ex. `ringbuf` med en double- eller triple-buffer) för att skicka IR-data till ljudmotorn.
    *   Modifiera `Convolver`-effekten att fungera som en **Master Effect** som kan ta emot och byta ut sin IR kontinuerligt utan ljudglapp.

### Phase 2: GUI - The "AWE View"

*Mål: Skapa ett intuitivt och visuellt 3D-gränssnitt.*

1.  **3D Viewport:**
    *   Skapa en ny, dedikerad `AppView::AcousticWorldEngine`.
    *   Implementera en grundläggande 3D-vy med pan/zoom/rotate-kontroller.
    *   Renderera den generiska 3D-meshen som utgör miljön.

2.  **Interaction:**
    *   Implementera **Drag-and-Drop** för att flytta objekt, ljudkällor och lyssnare i 3D-rymden.
    *   Skapa UI-element för att justera avstånd och höjd för stereo-par (källor/lyssnare).
    *   Skapa ett UI för att applicera och redigera avancerade material på olika ytor.

3.  **Real-time Visualization:**
    *   Implementera en visualisering av ljudstrålarna från ray tracern. För prestanda, visualisera endast ett fåtal (t.ex. de 100 första) strålarna.

### Phase 3: Interactive Objects & Physics

*Mål: Göra världen levande och responsiv.*

1.  **Object Placement:**
    *   Implementera möjligheten att placera primitiva former (Sfär, Cylinder, Kub) inuti den större miljön.
2.  **Sympathetic Resonators & Interaction Loop:**
    *   Implementera `Sympathetic Harp` och `Kinetic Windchimes` som kan kopplas till dessa objekt.
    *   Designa en feedback-loop i `awe_engine` där resonatorer kan excitera varandra.
3.  **Dynamic Physics:**
    *   Implementera **Doppler Shift** och **Surface Diffusion**.
4.  **Modal Membrane:**
    *   Forska och implementera en 2D Waveguide-modell som kan placeras på en yta.

### Phase 4: External Modulation

*Mål: Koppla samman AWE med resten av synthen.*

1.  **Parameter-brygga:**
    *   Skapa en trådsäker kommunikationskanal från `SynthEngine` till `awe_engine`.
    *   Exponera AWE-parametrar (t.ex. `room_size`, `object1_x_pos`, `material_diffusion`) som modulationsdestinationer.
2.  **Mod Matrix Integration:**
    *   Lägg till de nya AWE-destinationerna i `Mod Matrix` i patch-vyn.
    *   Ljudmotorn skickar modulerade parametervärden till `awe_engine` på varje block.
