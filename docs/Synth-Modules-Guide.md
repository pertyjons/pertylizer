# Synth Modules Guide

This document provides a detailed reference for all available synth modules in the `synth_modules` crate. Each section describes a module's purpose, its inputs, outputs, and parameters, and includes a simple schematic to illustrate its function.

---
## Oscillator
**Kategori:** Oscillator

Oscillatorn är den primära ljudkällan i en synt. Den genererar en kontinuerlig, periodisk vågform med en specifik tonhöjd (frekvens). Denna råa ljudsignal formas sedan vidare av andra moduler som filter och förstärkare.

Denna oscillator är "band-limited", vilket är en teknisk term som betyder att den använder smarta matematiska tekniker (PolyBLEP/PolyBLAMP) för att undvika digital distorsion (aliasing) vid höga frekvenser. Resultatet är ett renare och mer "analogt" ljud.

### Schematiskt flödesschema
```
  [CV-ingångar]
 (fm, pm, pwm)
       │
       ▼
┌──────────────┐   ┌─────────▶ (Audio) out (Mono mix)
│  Oscillator  │───┤
│ (Waveform,   │   ├─────────▶ (Audio) out_l (Stereo Vänster)
│  Unison...)  │───┤
└──────────────┘   └─────────▶ (Audio) out_r (Stereo Höger)
       ▲
       │
   [Gate-ingång]
      (sync)
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `fm` | CV | **Frequency Modulation**: Styr tonhöjden med en extern signal. Används för allt från subtilt vibrato (med en LFO) till komplexa, metalliska ljud (med en annan oscillator). |
| `pm` | CV | **Phase Modulation**: Liknar FM, men modulerar vågformens fas istället för frekvens. Ger ofta ett lite annorlunda, ibland ljusare, FM-ljud. |
| `pwm` | CV | **Pulse Width Modulation**: Styr bredden på `Pulse`-vågformen. Skapar en klassisk "sävlig" eller "tunn" effekt som är vanlig i analog syntes. |
| `cross_mod` | CV | **Cross-Modulation**: En annan typ av frekvensmodulering, ofta från en annan oscillator, för att skapa komplexa och aggressiva klanger. |
| `sync` | Gate | **Hard Sync**: Återställer vågformens cykel när den tar emot en puls. Om en oscillator synkas till en annan med lägre frekvens, tvingas den anpassa sin tonhöjd, vilket skapar karaktäristiska "riviga" övertoner. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Huvudutgången. Om Unison-läget används är detta en monomix av alla unison-röster. |
| `out_l` | Audio | Vänster stereoutgång. Används främst med Unison-läget för att skapa en bred stereobild. |
| `out_r` | Audio | Höger stereoutgång. Används främst med Unison-läget för att skapa en bred stereobild. |

### Parametrar

#### Huvudparametrar
*   **Waveform**: Väljer den grundläggande vågformen för ljudet. Varje vågform har en unik klang.
    *   `Sine`: Ren, mjuk ton utan övertoner. Som en stämgaffel.
    *   `Triangle`: Något ljusare än sinus, liknar en flöjt.
    *   `Sawtooth`: Rik på övertoner, ljus och "brassig". En klassisk synth-vågform.
    *   `Square`: Ihålig och nasal klang, likt en klarinett.
    *   `Pulse`: En variant av fyrkantsvåg där bredden kan justeras med `Pulse Width`.
*   **Frequency**: Oscillatorns grundtonhöjd, angiven i Hertz.
*   **Detune**: Finjusterar tonhöjden upp eller ner med upp till en halvton (100 cents). Används för att skapa en lätt "ostämd" effekt mellan två oscillatorer.
*   **Pulse Width**: Justerar pulsbredden för `Pulse`-vågformen. Värdet 0.5 (50%) ger en perfekt fyrkantsvåg. Andra värden ger ett tunnare, mer nasalt ljud.
*   **Level**: Styr den övergripande volymen på oscillatorn.

#### FM-parametrar
*   **FM Mode**: Ställer in hur `fm`-ingången tolkar signalen.
    *   `Exponential`: Förväntar sig en "1V/oktav"-signal, där varje volt dubblar frekvensen. Standard för tonhöjds-CV.
    *   `Linear`: Adderar eller subtraherar frekvensen linjärt. Används för klassisk FM-syntes.
*   **FM Amt**: En "attenuverter" som skalar FM-signalen. Vid `1.0` är signalen oförändrad, vid `0.0` har den ingen effekt, och vid `-1.0` är den inverterad.
*   **X-Mod**: Styr mängden cross-modulation från `cross_mod`-ingången.

#### Unison-parametrar
Unison-läget skapar flera kopior (röster) av oscillatorn för varje spelad not, vilket ger ett tjockare och fylligare ljud.
*   **Unison**: Antal röster som ska spelas, från 1 (av) till 7.
*   **Uni Detune**: Hur mycket rösterna ska stämmas isär. Ett högre värde ger ett mer "svävande" och körliknande ljud.
*   **Uni Spread**: Sprider ut unison-rösterna i stereobilden för att skapa en bredare ljudbild. Vid 0.0 är alla röster i mitten (mono).
*   **Uni Phase**: Slumpmässig startfas för varje unison-röst vid varje ny not. Ett högt värde gör att varje not låter aningen annorlunda och mer "levande".
---
## Additive Oscillator
**Kategori:** Oscillator

Additiv syntes är en ljudskapande metod som bygger komplexa ljud genom att summera ett stort antal enkla sinusvågor (kallade "övertoner" eller "partialer"). Istället för att forma en rik vågform med ett filter (som i subtraktiv syntes), bygger man här klangen från grunden, likt hur en orgel skapar ljud.

Denna modul ger dig kontroll över ett spektrum av 32 övertoner genom ett fåtal kraftfulla, övergripande parametrar.

### Schematiskt flödesschema
```
[Freq CV]───▶ Styr grundtonen
               │
               ▼
        ┌────────────────┐
        │ Additive Osc.  │
        │ (32 sinusvågor)│
        │                ├─▶ (Audio) out
        │ Tilt, Odd/Even,│
        │ Bright, Stretch..
        └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `freq_cv` | CV | **Frequency Control Voltage**: Styr den fundamentala tonhöjden för hela det harmoniska spektrumet. Följer "1V/oktav"-standarden. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den mixade monosignalen från alla 32 sinusvågor. |

### Parametrar

Dessa parametrar styr den övergripande klangen genom att justera styrkan på de 32 övertonerna samtidigt.

*   **Tilt**: Justerar den spektrala balansen. Tänk dig en EQ-ratt som "lutar" hela spektrumet. Ett lågt värde förstärker basharmonierna och dämpar diskanten (mörkare ljud), medan ett högt värde gör tvärtom (ljusare ljud).
*   **Odd/Even**: Balanserar styrkan mellan udda (1, 3, 5...) och jämna (2, 4, 6...) övertoner.
    *   `0.0`: Endast udda övertoner, vilket ger ett ihåligt, fyrkantsvåg-liknande ljud.
    *   `0.5`: Jämn balans.
    *   `1.0`: Främst jämna övertoner, vilket kan ge ett oktav-liknande, speciellt ljud.
*   **Brightness**: En mer fokuserad kontroll för de allra högsta, luftigaste övertonerna. Användbar för att lägga till eller ta bort "skimmer" från ljudet.
*   **Stretch**: Skapar inharmonicitet genom att "sträcka ut" avståndet mellan övertonerna så att de inte längre är perfekta multiplar av grundtonen. Låga värden kan ge en subtil pianoliknande karaktär, medan höga värden skapar dissonanta, klock- eller metall-liknande ljud.
*   **Randomize**: Styr hur mycket fasen på varje överton slumpas vid varje ny not.
    *   `0.0`: Alla övertoner startar i perfekt synk, vilket ger ett statiskt och "dött" ljud.
    *   Högre värden: Varje not får en unik, subtilt annorlunda karaktär, vilket gör ljudet mer levande och organiskt.
*   **Level**: Styr den slutgiltiga volymen på modulen.
---
## Granular Oscillator
**Kategori:** Oscillator

Granulär syntes är en avancerad ljudskapande metod som bygger komplexa texturer och ljudlandskap. Istället för att spela en kontinuerlig vågform, skapar den ljud genom att spela upp och mixa ett stort antal mycket korta ljudfragment, kallade "grains" (korn). Varje korn är en liten bit av en källvågform, omsluten av en envelope för att undvika klickar.

Denna modul är utmärkt för att skapa täta, evolverande ljudmoln, atmosfäriska pads och glitchiga texturer.

### Schematiskt flödesschema
```
        ┌────────────────┐
        │ Granular Osc.  │
        │ (32 Grains)    │
        │                ├─▶ (Audio) out
        │  Source Wave,  │
        │  Density, Size...
        └────────────────┘
```
Denna modul har inga CV-ingångar och styrs helt av sina interna parametrar.

### Ingångar (Inputs)
*   *Inga*

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den mixade monosignalen från alla 32 aktiva korn. |

### Parametrar

*   **Grain Size**: Längden på varje enskilt korn, i millisekunder. Korta korn (5-20ms) skapar surrande, nästan tonala texturer. Längre korn (50-500ms) kan skapa asynkrona, lager-på-lager-ekon.
*   **Density**: Styr hur ofta nya korn skapas. Låg densitet ger glesa, distinkta händelser. Hög densitet skapar ett tätt och kontinuerligt ljudmoln.
*   **Position**: Startpositionen i källvågformen varifrån kornen hämtas. Att svepa denna parameter är som att "skrubba" genom en ljudfil och är nyckeln till att skapa rörelse i ljudet.
*   **Pos Spread**: Introducerar slumpmässighet till startpositionen för varje nytt korn. Ett högt värde gör att kornen hämtas från ett större område av källvågformen, vilket skapar en mer varierad och kaotisk textur.
*   **Pitch Spread**: Introducerar slumpmässighet till tonhöjden för varje nytt korn. Skapar en dissonant, körliknande effekt som kan sträcka sig från subtil ostämdhet till atonala moln.
*   **Pan Spread**: **(Notera: För närvarande endast teoretisk)** Parametern är designad för att introducera slumpmässig stereopanorering för varje korn, men eftersom modulen för närvarande bara har en monoutgång har den ingen hörbar effekt.
*   **Freeze**: När denna är aktiv "fryser" modulen den nuvarande `Position`, vilket gör att nya korn upprepade gånger tas från samma lilla sektion. Detta skapar en hackande, loop-liknande effekt.
*   **Window**: Formen på den envelope som appliceras på varje korn för att förhindra klickar.
    *   `Hann`: En mjuk, standard fönsterfunktion.
    *   `Gaussian`: Ännu mjukare, med en klockformad kurva.
    *   `Trapezoid`: Skarpare kanter, vilket kan ge en något mer "klickig" eller perkussiv karaktär.
*   **Source**: Den underliggande vågform som kornen skapas från. Välj mellan `Saw`, `Sine`, `Square`, `Triangle` och `Noise`.
*   **Level**: Den övergripande utgångsvolymen för modulen.
---
## Math Oscillator
**Kategori:** Oscillator

Detta är en digital "schweizerkniv" till oscillator. Istället för traditionella vågformer innehåller den 19 olika matematiska algoritmer för att generera ljud. Detta gör den extremt mångsidig för att skapa allt från klassiska FM-ljud och fysisk modellering till kaotiska och glitchiga texturer.

### Schematiskt flödesschema
```
        [CV-ingångar]
  (fm, mod_a, mod_b)
             │
             ▼
      ┌───────────────┐
      │ Math Oscillator │
      │ (19 Algoritmer) ├─▶ (Audio) out
      │ (Param A/B/C)   │
      └───────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `fm` | CV | **Frequency Modulation**: Styr grundfrekvensen exponentiellt (1V/oktav). |
| `param_a` | CV | Modulerar **Param A**. |
| `param_b` | CV | Modulerar **Param B**. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Monosignalen från den valda algoritmen. |

### Parametrar

*   **Algorithm**: Väljer vilken av de 19 ljudgenereringsalgoritmerna som ska användas.
*   **Frequency**: Ställer in grundfrekvensen (tonhöjden).
*   **Param A / B / C**: Tre generella parametrar (0.0 till 1.0) vars funktion helt beror på den valda algoritmen. De är till för att utforska och forma ljudet från varje algoritm.
*   **Level**: Den övergripande utgångsvolymen.

### Algoritmer
Algoritmerna kan grovt delas in i tre kategorier:

#### 1. Fasbaserade (förutsägbara och repeterbara)
Dessa algoritmer genererar ljud baserat på den nuvarande fasen, likt en vanlig oscillator.
*   **SineFM**: Klassisk FM-syntes. `A`=modulationsindex, `B`=modulatorförhållande.
*   **TanChaos**: Hård, kaotisk distorsion.
*   **SuperSaw**: Flera ostämda sågtandsvågor för ett tjockt, fylligt ljud.
*   **BitWise**: Digitala glitchljud via bit-operationer.
*   **WaveFolder**: Vik-distorsion i "West Coast"-stil.
*   **Formant**: Simulerar vokalliknande ljud.
*   **PhaseDist**: Fasdistorsion i stil med Casio CZ-syntar.
*   **Metallic**: Ringmodulationsliknande metalliska ljud.
*   **Fractal**: Komplexa, självliknande vågformer.
*   **Chebyshev**: Vågforms-distorsion med polynom.
*   **Walsh**: Summa av fyrkantsvågor för digitala texturer.
*   **Pulsar**: Ljud genererat i korta pulser.
*   **Shepard**: Skapar ilussionen av en oändligt stigande/fallande ton.

#### 2. Iterativa/Kaotiska (utvecklas över tid)
Dessa algoritmer har ett internt minne och deras ljud kan utvecklas på oförutsägbara sätt.
*   **Bytebeat**: Genererar komplexa mönster från enkla matematiska formler, populärt i demoscenen.
*   **Lorenz**: Kaotisk generator baserad på Lorenz-attraktorn. Perfekt för slumpmässiga, evolverande CV-signaler eller brus.
*   **Logistic**: En annan kaotisk generator, baserad på den logistiska kartan.
*   **FeedbackFM**: FM-syntes där oscillatorn modulerar sig själv. Skapar lätt kaotiska och oförutsägbara resultat.
*   **Vosim**: Simulerar röstklanger med hjälp av fyrkantiga sinuspulser.

#### 3. Buffertbaserade (fysisk modellering)
*   **KarplusStrong**: Simulerar en knäppt sträng. `A` styr dämpning, `B` styr "burst"-ljudets karaktär.
---
## Sub Oscillator
**Kategori:** Oscillator

En sub-oscillator är en hjälpgenerator vars enda syfte är att addera tyngd och "fett" till ett ljud. Den följer automatiskt tonhöjden från den spelade noten, men en eller två oktaver lägre. Detta är ett klassiskt knep för att skapa fylliga basljud utan att behöva använda en av sina huvudsakliga oscillatorer.

### Schematiskt flödesschema
```
(Spelad not) ───┐
                ▼
        ┌────────────┐
        │  Sub Osc   ├─▶ (Audio) out
        │ (-1/-2 Oct)│
        └────────────┘
```

### Ingångar (Inputs)
*   *Inga.* Modulen följer automatiskt den inkommande noten för den röst den tillhör.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Monosignalen från sub-oscillatorn. |

### Parametrar

*   **Waveform**: Vågformen för sub-oscillatorn. Valet påverkar karaktären på basljudet.
    *   `Sine`: En ren, fundamental ton. Perfekt för djup och ren sub-bas.
    *   `Square`: En fylligare ton med några övertoner, vilket kan hjälpa basen att höras även i mindre högtalarsystem.
    *   `Pulse25`: En smalare pulsvåg som ger en tunnare, mer "nasal" baskaraktär.
*   **Octave**: Väljer hur många oktaver under huvudnoten som sub-oscillatorn ska spela.
    *   `-1`: En oktav under. Det vanligaste valet för de flesta basljud.
    *   `-2`: Två oktaver under. Används för extremt djup sub-bas som mer känns än hörs.
*   **Level**: Styr volymen på sub-oscillatorn. Används för att mixa in precis rätt mängd basstöd.
---
## Wavetable Oscillator
**Kategori:** Oscillator

En wavetable-oscillator skapar ljud genom att spela upp korta, en-cykel långa vågformer som lagras i en "vågtabell" (wavetable). Dess styrka ligger i att den kan mjukt och dynamiskt växla, eller "morpha", mellan olika vågformer i tabellen. Detta skapar komplexa, evolverande klanger som är svåra att uppnå med traditionella oscillatorer.

### Schematiskt flödesschema
```
        [CV-ingångar]
      (fm, pos_cv)
             │
             ▼
      ┌───────────────┐
      │  Wavetable    │
      │  Oscillator   ├─▶ (Audio) out
      │ (Table, Pos)  │
      └───────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `fm` | CV | **Frequency Modulation**: Modulerar tonhöjden (1V/oktav). |
| `pos_cv` | CV | **Position CV**: Modulerar `Position`-parametern. Detta är den viktigaste ingången för att skapa ljudets rörelse och karaktär. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Monosignalen från oscillatorn. |

### Parametrar

*   **Table**: Väljer vilken "vågtabell" (bank av vågformer) som ska användas. Varje tabell har ett unikt sound.
    *   `Basic`: En mjuk morph från sinusvåg till sågtand och fyrkantsvåg.
    *   `Harmonics`: Börjar som en ren sinuston och adderar successivt upp till 32 övertoner.
    *   `PWM`: Simulerar den klassiska pulsbreddsmodulations-effekten.
    *   `Formant`: Innehåller vågformer som liknar mänskliga vokaler (a, e, i, o, u).
    *   `Digital`: En samling av mer aggressiva, bullriga och digitalt klingande vågformer.
    *   `Warm`: Mjukare, mer "analogt" mättade vågformer.
*   **Position**: Den viktigaste parametern. Den "sveper" genom den valda vågtabellen och morphar mellan dess olika vågformer. Att modulera denna med en LFO eller envelope är kärnan i wavetable-syntes.
*   **Detune**: Finjusterar tonhöjden i cents (1/100-dels halvton).
*   **Octave**: Transponerar tonhöjden upp eller ner i hela oktaver (-2 till +2).
*   **Level**: Den övergripande utgångsvolymen.
---
## Filter
**Kategori:** Filter

Filtret är en av de viktigaste delarna i subtraktiv syntes. Det formar klangen på ett ljud genom att ta bort (filtrera) eller förstärka vissa frekvenser. Genom att svepa filtrets `Cutoff`-frekvens skapas den klassiska, uttrycksfulla "wah-wah"-effekten som definierar mycket av elektronisk musik.

Denna modul är ett mångsidigt "State Variable Filter" (SVF) som dessutom kan emulera karaktären hos flera kända analoga filterkretsar.

### Schematiskt flödesschema
```
        [CV-ingångar]
(cutoff_cv, res_cv)
           │
           ▼
    ┌────────────┐
──▶ │     in     │
    │   Filter   ├─▶ (Audio) out
    │ (SVF)      │
    └────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `in` | Audio | Ljudsignalen som ska filtreras. |
| `cutoff_cv` | CV | Modulerar `Cutoff`-frekvensen. Koppla en envelope eller LFO här för att skapa rörelse i ljudet. |
| `res_cv` | CV | Modulerar `Resonance`. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den filtrerade ljudsignalen. |

### Parametrar

*   **Model**: Väljer filtrets "karaktär" genom att emulera olika analoga kretsar.
    *   `Standard`: Ett rent och precist digitalt filter.
    *   `Fluid`: Inspirerat av Oberheim-filter, med en mjuk och "flytande" karaktär. Har en unik `Morph`-parameter.
    *   `Screamer`: Inspirerat av MS-20-filter, känt för sin aggressiva och "skrikande" resonans.
    *   `Acid`: Inspirerat av Steiner-Parker-filter (likt TB-303), perfekt för "squelchy" acid-basljud.
*   **Morph**: **(Endast för `Fluid`-modellen)** Morphar mjukt mellan Lowpass, Bandpass, Highpass och Notch, vilket skapar unika svepande effekter.
*   **Type**: Väljer den grundläggande filterfunktionen.
    *   `Lowpass`: Tar bort höga frekvenser, lämnar basen. Det vanligaste och mest fundamentala filter-läget.
    *   `Highpass`: Tar bort basen, lämnar de höga frekvenserna.
    *   `Bandpass`: Tar bort både bas och diskant, lämnar bara ett smalt frekvensband i mitten.
    *   `Notch`: Motsatsen till bandpass; tar bort ett smalt frekvensband.
    *   `Peak`: Förstärker ett smalt frekvensband.
    *   `LowShelf` / `HighShelf`: Som bas/diskant-kontrollerna på en stereo, förstärker eller dämpar allt under/över cutoff-frekvensen.
*   **Cutoff**: Den viktigaste parametern. Bestämmer vid vilken frekvens filtret börjar arbeta. Att svepa denna är det som skapar den klassiska filtereffekten.
*   **Resonance**: Förstärker frekvenserna precis vid cutoff-punkten. Ger ljudet en "ringande" eller "visslande" karaktär. Vid höga värden kan filtret börja själv-oscilliera och skapa en egen ton.
*   **Key Track**: Får cutoff-frekvensen att följa tonhöjden på den spelade noten. Detta gör att högre noter låter ljusare och mer naturliga.
*   **CV Amt**: Skalar eller inverterar signalen som kommer in på `cutoff_cv`-ingången.
*   **Drive**: Överstyr insignalen in i filtret, vilket skapar en varm, analog-liknande distorsion och mättnad.
---
## Amplifier (VCA)
**Kategori:** Amplifier

En "Voltage Controlled Amplifier" är syntvärldens volymkontroll. Dess primära uppgift är att forma den dynamiska profilen för ett ljud. Vanligtvis kopplar man en envelope-generator till `CV`-ingången för att ge en ton en definierad attack (hur snabbt ljudet startar) och release (hur länge det ringer ut).

### Schematiskt flödesschema
```
          [CV-ingångar]
         (cv, pan_cv)
               │
               ▼
      ┌────────────────┐
(in)──┤                │
      │   Amplifier    ├─▶(left)
(in_l)─┤     (VCA)      ├─▶(right)
      │                ├─▶(out)
(in_r)──┤                │
      └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `in` | Audio | Mono-ljudingång. |
| `in_l` | Audio | Vänster stereokanal in. Om denna används, ignoreras `in`. |
| `in_r` | Audio | Höger stereokanal in. Om denna används, ignoreras `in`. |
| `cv` | CV | **Control Voltage**: Huvudingången för att styra volymen. En signal från 0.0 till 1.0 skalar volymen från tystnad till full styrka. |
| `pan_cv` | CV | Modulerar stereopanoreringen. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `left` | Audio | Vänster stereoutgång. |
| `right` | Audio | Höger stereoutgång. |
| `out` | Audio | En monomix av vänster och höger kanal. |

### Parametrar

*   **Level**: Grundvolymen. Denna multipliceras med signalen från `cv`-ingången. Kan ställas över 1.0 för att ge en volym-boost.
*   **Pan**: Grundinställningen för stereopanorering, från helt vänster (-1.0) till helt höger (1.0).
*   **CV Bipolar**: Ändrar hur `cv`-ingången beter sig.
    *   **Av (standard)**: CV-signalen behandlas som unipolär (0.0 till 1.0). Detta är normalt VCA-beteende för volymkontroll.
    *   **På**: CV-signalen behandlas som bipolär (-1.0 till 1.0). Detta möjliggör "ringmodulation"-liknande effekter, där en negativ ljudvågs fas inverteras.
---
## Envelope (ADSR)
**Kategori:** Envelope

En Envelope-generator är avgörande för att forma ljudets dynamik över tid. Den skapar en kontrollsignal som vanligtvis ändras när en ton spelas. Den vanligaste typen är en ADSR-envelope (Attack, Decay, Sustain, Release), vilken används för att styra en VCA (för volym) eller ett filters cutoff-frekvens. Den definierar hur ett ljud startar, dess kropp, och hur det tonar ut.

### Schematiskt flödesschema
```
 (Gate)───▶┌──────────┐
          │ Envelope ├─▶ (CV) out
 (Vel)────▶│  (ADSR)  │
          └──────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `gate` | Gate | En signal (t.ex. från ett tangenttryck) som talar om för envelopen att starta sin cykel. |
| `velocity` | CV | Tar emot anslagshastigheten från den spelade noten (hur hårt tangenten trycktes ned). Kan användas för att skala envelopens utgång. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | CV | Utgångens kontrollspänning (0.0 till 1.0) som representerar envelopens form. |

### Parametrar

*   **Attack**: Tiden det tar för envelopen att gå från noll till sin maximala nivå när en not först spelas. En kort attack är perkussiv (som en trumma), medan en lång attack skapar en "sväll"-effekt.
*   **Decay**: Tiden det tar att gå från den maximala attacknivån ner till sustainnivån.
*   **Sustain**: Nivån envelopen håller sig på så länge noten hålls ned (after the attack and decay phases are complete). Ett värde på 1.0 betyder att den håller full volym, medan 0.0 betyder att ljudet helt tonar ut även om tangenten hålls ned.
*   **Release**: Tiden det tar för envelopen att tona ut till noll efter att noten släppts. En lång release skapar en "reverb-liknande" svans.
*   **Vel Sens (Velocity Sensitivity)**: Kontrollerar hur mycket notens anslagshastighet påverkar envelopens utgångsnivå. Vid 1.0 ger ett mjukt tangenttryck ett tyst ljud och ett hårt tryck ett högt ljud. Vid 0.0 har alla noter samma volym oavsett hur hårt de spelas.
*   **Atk Curve / Dec Curve / Rel Curve**: Dessa parametrar ändrar formen på respektive fas från linjär till exponentiell eller logaritmisk, vilket påverkar hur "rapp" eller "mjuk" de känns. Ett värde på 0.0 är linjärt. Negativa värden är exponentiella (snabbare start, långsammare slut), och positiva värden är logaritmiska (långsammare start, snabbare slut).
---
## LFO (Low Frequency Oscillator)
**Kategori:** Modulator

En LFO (Low Frequency Oscillator) är en oscillator som arbetar på mycket låga frekvenser, vanligtvis under det hörbara spektrumet. Den används inte för att skapa ljud direkt, utan för att modulera andra parametrar i synthen, vilket skapar rörelse och variation. Vanliga användningsområden inkluderar vibrato (modulerar tonhöjd), tremolo (modulerar volym) och wah (modulerar filtercutoff).

### Schematiskt flödesschema
```
        ┌─────────┐
        │   LFO   ├─▶ (CV) out
        │ (Wave,  │
(Rate)─▶│ Depth)  │
(Depth)▶│         │
        └─────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `rate_cv` | CV | Modulerar LFO:ns frekvens (hastighet). |
| `depth_cv` | CV | Modulerar LFO:ns djup (amplitud). |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | CV | LFO:ns utgångssignal (vanligtvis en bipolär vågform, -1.0 till 1.0). |

### Parametrar

*   **Waveform**: Väljer LFO:ns vågform. Varje vågform har en unik modulationskaraktär.
    *   `Sine`: Mjuk och jämn modulation (t.ex. klassiskt vibrato).
    *   `Triangle`: Linjär modulation upp och ner.
    *   `Saw`: Linjär ramp upp, snabbt fall.
    *   `Square`: Omedelbara byten mellan max och min (t.ex. trigger-liknande effekter).
    *   `Random Sample & Hold`: Håller ett slumpmässigt värde under varje cykel.
*   **Rate**: LFO:ns hastighet, i Hertz. Från mycket långsamt (bråkdelar av Hz) till snabbare (några Hz).
*   **Depth**: LFO:ns amplitud, eller hur intensiv modulationen är. Kontrollerar det maximala utslag från -1.0 till 1.0.
*   **Offset**: En DC-offset som läggs till LFO-signalen. Kan användas för att skifta modulationsområdet.
---
## MSEG (Multi-Stage Envelope Generator)
**Kategori:** Envelope

En MSEG är en avancerad envelope-generator som tillåter användaren att rita fritt egna, komplexa enveloper med flera "segment" eller "steg". Istället för de fasta ADSR-faserna kan en MSEG ha ett godtyckligt antal punkter som definierar formen på modulationssignalen. Varje punkt kan ha en fritt justerbar kurva, vilket möjliggör skapandet av mycket organiska och detaljerade modulationsmönster.

### Schematiskt flödesschema
```
 (Gate)───▶┌─────────┐
          │   MSEG  ├─▶ (CV) out
 (Loop)───▶│ (Points,│
          │ Curves) │
          └─────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `gate` | Gate | En trigger-signal som startar MSEG-cykeln. |
| `loop_cv` | CV | En kontrollspänning som kan aktivera eller inaktivera loop-funktionen. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | CV | Utgångs-CV-signalen som genereras av MSEG. |

### Parametrar

*   **Points**: Antalet segment i envelopen. Varje segment definieras av två punkter.
*   **Rate**: Hastigheten med vilken envelopen spelas upp.
*   **Loop Mode**: Bestämmer hur envelopen upprepas.
    *   `Off`: Spelas bara en gång.
    *   `Loop`: Loopar hela envelopen.
    *   `Loop Sustain`: Loopar ett specifikt avsnitt av envelopen tills noten släpps.
*   **Loop Start/End**: Definierar vilket segment som ska loopas i `Loop Sustain`-läge.
*   **Curve**: För varje segment kan kurvans form justeras från linjär till exponentiell eller logaritmisk.
*   **Level**: Utgångsnivån för MSEG.
---
## Noise
**Kategori:** Noise

Brus är en grundläggande ljudkälla i syntes som inte har en definierad tonhöjd. Det används för att lägga till "luft", "sand", "hiss", eller perkussiva element i ett ljud. Denna modul erbjuder olika typer av brus, var och en med sin egen spektrala karaktär, vilket gör den användbar för att skapa allt från virvlande vind till digitala artefakter.

### Schematiskt flödesschema
```
        ┌─────────┐
        │  Noise  ├─▶ (Audio) out
(Level)─▶│ (Type,  │
        │ Filter) │
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga.*

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den genererade brussignalen. |

### Parametrar

*   **Type**: Väljer brustyp, var och en med en unik frekvensfördelning.
    *   `White`: Jämn fördelning av alla hörbara frekvenser, låter som statiskt brus.
    *   `Pink`: Liknar vitt brus men med mer energi i de lägre frekvenserna, låter "varmare" och mer naturligt.
    *   `Brown`: Ännu mer bas, låter som ett djupt muller.
    *   `Digital`: Ger ett "glitchigare" och mer "grusigt" brus, som kan låta som gamla videospel.
    *   `Crackle`: Simulerar knastrande ljud, som från en gammal vinylskiva.
*   **Color**: Ändrar brussignalen genom att applicera ett resonansfilter. Kan svepa från lågpass till högpass.
*   **Level**: Den övergripande utgångsvolymen.
---
## Output
**Kategori:** Utility

Output-modulen fungerar som syntens master-utgång. Alla ljudsignaler som ska höras routas genom denna modul. Den ger grundläggande kontroll över den slutgiltiga volymen och stereopanoreringen. Dessutom har den en inbyggd brickwall-limiter för att förhindra överstyrning och skydda dina öron och högtalare.

### Schematiskt flödeschema
```
(in_l)──▶┌─────────┐
        │ Output  ├─▶(out_l)
(in_r)──▶│ (Volume,│
        │  Pan,   ├─▶(out_r)
(gain)─▶│ Limiter)│
        └─────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `in_l` | Audio | Vänster stereokanal in. |
| `in_r` | Audio | Höger stereokanal in. |
| `gain_cv` | CV | Modulerar huvudvolymen. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Volume**: Huvudvolymen för hela synten.
*   **Pan**: Stereopanorering av den slutgiltiga signalen.
*   **Limiter**: Aktiverar eller inaktiverar den inbyggda brickwall-limitern.
*   **Ceiling**: Ställer in tröskeln för limitern. Ingen signal kommer att överstiga detta värde.
*   **Release**: Tiden det tar för limitern att återgå till normal förstärkning efter att den har aktiverats.
---
## Mod Matrix (Modulation Matrix)
**Kategori:** Utility

Modulationsmatrisen är syntens centrala kopplingspanel. Den låter dig visuellt och flexibelt koppla modulationskällor (t.ex. LFO:er, Enveloper, tangentanslag) till destinationsparametrar (t.ex. oscillatorfrekvens, filtercutoff, pan). Den är uppbyggd som ett rutnät där varje "cell" representerar en koppling mellan en källa och en destination, och där intensiteten för denna koppling kan justeras.

### Schematiskt flödesschema
```
[Mod Sources]─┐   ┌──▶[Mod Destinations]
              ▼   │
        ┌─────────────┐
        │  Mod Matrix ├
        │ (Sources,   │
        │ Destinations)
        └─────────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Modulationskällor och destinationer är definierade internt.

### Utgångar (Outputs)
*   *Inga explicita ljud- eller CV-utgångar.* Modulationsmatrisen "skickar" direkt modulationsvärden till de definierade destinationerna.

### Parametrar

*   **Grid Size**: Storleken på matrisen (t.ex. 2x2, 4x4, 8x8). Detta avgör hur många modulationskällor som kan kopplas till hur många destinationer.
*   **Source X / Destination Y / Amount**: För varje cell i matrisen väljer du:
    *   **Source**: Vilken modulationskälla som ska användas (t.ex. LFO1, Env2, Velocity).
    *   **Destination**: Vilken parameter som ska moduleras (t.ex. Osc1 Freq, Filter1 Cutoff).
    *   **Amount**: Hur starkt modulationskällan påverkar destinationen. Kan vara positiv eller negativ (inverterad modulation).
---
## Ring Mod (Ring Modulator)
**Kategori:** Mixer

Ringmodulatorn är en effekt som multiplicerar två insignaler med varandra. Resultatet är ett ljud som innehåller sum- och differensfrekvenserna av originalsignalerna, men *inte* originalsignalerna själva. Detta skapar ofta metalliska, klockliknande eller robotaktiga klanger.

### Schematiskt flödesschema
```
(in_a)──▶┌─────────┐
        │ Ring Mod├─▶ (Audio) out
(in_b)──▶│ (Mix)   │
        └─────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `in_a` | Audio | Den första insignalen. |
| `in_b` | Audio | Den andra insignalen. |
| `mix_cv` | CV | Modulerar dry/wet-mixen. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den ringmodulerade utgångssignalen. |

### Parametrar

*   **Mix**: Kontrollerar balansen mellan dry (originalsignaler) och wet (ringmodulerad signal).
*   **Level**: Utgångsvolymen för modulen.
---
## Envelope Follower
**Kategori:** Analysator

En Envelope Follower analyserar volymen (envelopen) av en inkommande ljudsignal och omvandlar den till en kontrollspänning (CV). Denna CV-signal kan sedan användas för att modulera andra parametrar i synthen. Ett vanligt exempel är att använda en trumloop som insignal, låta Envelope Followern extrahera dess rytmiska profil, och sedan använda den profilen för att styra ett filter eller en LFO, vilket skapar ett "ducka"- eller "sidechain"-liknande effekt.

### Schematiskt flödesschema
```
(in)──▶┌────────────────┐
      │ Env. Follower  ├─▶ (CV) out
      │ (Attack, Decay)│
      └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `in` | Audio | Ljudsignalen som ska analyseras. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | CV | Utgångs-CV-signalen som representerar insignalens volym. |

### Parametrar

*   **Attack**: Tiden det tar för Envelope Followerns utgång att reagera på en ökning i insignalens volym. En snabb attack följer snabbt transienter, medan en lång attack ger en mjukare, mer utjämnad respons.
*   **Release**: Tiden det tar för Envelope Followerns utgång att reagera på en minskning i insignalens volym. En snabb release gör att den snabbt faller när ljudet tystnar, en lång release håller upp CV-signalen längre.
*   **Gain**: Förstärker den inkommande ljudsignalen innan analysen, vilket påverkar känsligheten.
---
## Body Resonance
**Kategori:** Fysisk Modellering

Body Resonance-modulen simulerar resonansen hos olika fysiska "kroppar" eller material, som trä, metall eller trumma. Genom att mata in en "exciter"-signal (t.ex. en kort puls eller brus) kan modulen skapa ljud som låter som om de slår an eller resonerar i dessa material. Detta är en form av fysisk modellering som kan användas för att skapa trum- och perkussionsljud, plockade strängar, klockor eller unika, resonanta texturer.

### Schematiskt flödesschema
```
(in)──▶┌────────────────┐
      │ Body Resonance ├─▶ (Audio) out
      │ (Type, Bright) │
      └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `in` | Audio | "Exciter"-signalen som "slår an" den simulerade kroppen. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den resonerande ljudsignalen. |

### Parametrar

*   **Type**: Väljer vilken typ av kropp som simuleras. Detta ändrar de grundläggande resonansegenskaperna.
    *   `Wood`: Låter som trä, med en viss värme och dämpning.
    *   `Metal`: Ger metalliska, ringande toner.
    *   `Drum`: Simulerar resonansen hos ett trumskinn.
    *   `Glass`: Ger en skör, glas-liknande resonans.
*   **Frequency**: Ställer in den grundläggande resonansfrekvensen för kroppen.
*   **Decay**: Kontrollerar hur snabbt resonansen klingar ut.
*   **Brightness**: Justerar klangfärgen på resonansen, ofta genom att ändra dämpningen av högre övertoner.
*   **Mix**: Dry/wet-mix för att blanda den resonerande signalen med originalet.
---
## Mechanical Noise
**Kategori:** Noise

Mechanical Noise-modulen genererar olika typer av "mekaniskt" brus, som klick, dämpning, tangenttryck eller hammarslag. Den är utformad för att lägga till realism och "smuts" till syntljud, särskilt användbart för att emulera akustiska instrument eller äldre, mekaniska syntar. Till skillnad från enkel vitt brus kan dessa ljud triggas synkroniserat med noter för att förstärka attacken eller releasen av ett ljud.

### Schematiskt flödesschema
```
 (Gate)───▶┌────────────────┐
          │ Mechanical     ├─▶ (Audio) out
          │ Noise (Type,   │
          │  Length)       │
          └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `gate` | Gate | En trigger-signal som aktiverar brusljudet. |
| `velocity` | CV | Styr intensiteten (volymen) av det genererade bruset. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Det genererade mekaniska bruset. |

### Parametrar

*   **Type**: Väljer typ av mekaniskt brus.
    *   `KeyDown`: Ljudet av ett tangenttryck.
    *   `KeyUp`: Ljudet när ett tangent släpps.
    *   `Hammer`: Ett simulerat hammarslag, som från ett piano.
    *   `Damper`: Ljudet av en dämpare som lyfts eller sänks, som i ett piano.
*   **Length**: Längden på brusljudet.
*   **Filter**: Ett resonansfilter som kan användas för att ändra klangfärgen på bruset.
*   **Level**: Utgångsvolymen för modulen.
---
## Keyboard Panner
**Kategori:** Utility

Keyboard Panner-modulen panorerar (placerar i stereobilden) ljud baserat på vilken tonhöjd som spelas på klaviaturen. Låga toner kan panoreras till vänster, höga toner till höger, eller vice versa. Detta skapar en naturlig och bred stereobild för pianon, stråkar eller andra instrument som traditionellt har ett brett frekvensspektrum över stereobilden.

### Schematiskt flödesschema
```
 (Note)───▶┌────────────────┐
          │ Keyboard Panner├─▶ (CV) pan_out
          │ (Range, Curve) │
          └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `note` | MIDI | Tar emot MIDI-tonhöjden för den spelade noten. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `pan_out` | CV | En kontrollspänning för panorering (-1.0 till 1.0) som kan kopplas till en förstärkares pan-parameter. |

### Parametrar

*   **Range**: Definierar vilket MIDI-notintervall som ska mappas över hela stereobilden. Till exempel, C1 till C6.
*   **Curve**: Bestämmer hur panoreringen sker inom det definierade intervallet. Kan vara linjär, logaritmisk eller exponentiell.
*   **Invert**: Inverterar panoreringen, så att låga toner går höger och höga toner går vänster.
*   **Level**: Skalar utgångs-CV-signalen.
---
## Euclidean Sequencer
**Kategori:** Sequencer

Euclidean Sequencer är en mönstergenerator som skapar rytmer baserat på den Euklidiska algoritmen. Den fördelar ett visst antal "pulser" (slag) så jämnt som möjligt över ett visst antal "steg". Detta är idealiskt för att skapa en mängd komplexa, men ändå organiska och musikaliska rytmer som ofta hittas i världsmusik och elektronisk musik, utan att behöva programmera varje enskilt slag manuellt.

### Schematiskt flödesschema
```
 (Clock)───▶┌────────────────┐
           │ Euclidean Seq  ├─▶ (Gate) out
 (Reset)───▶│ (Steps, Pulses)│
           └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `clock` | Gate | En klockpuls som driver sekvensern framåt ett steg i taget. |
| `reset` | Gate | En puls som återställer sekvensern till början av mönstret. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Gate | En utgående gate-signal som representerar de triggade pulserna. |

### Parametrar

*   **Steps**: Det totala antalet steg i sekvensen.
*   **Pulses**: Antalet pulser (slag) som ska fördelas över stegen.
*   **Offset**: Skiftar mönstrets startpunkt.
*   **Length**: Längden på de utgående gate-pulserna.
---
## Turing Machine
**Kategori:** Sequencer

Turing Machine-modulen är en slumpmässig men kontrollerbar sekvensgenerator, inspirerad av en klassisk analog modul. Den genererar en serie port- och CV-signaler som är "kvantiserade" till en skala. Det unika med denna modul är dess förmåga att introducera "slumpmässighet" och "mutation" i sekvensen över tid, samtidigt som användaren kan "låsa" delar av mönstret. Detta gör den idealisk för att skapa intressanta, evolverande melodier, arpeggion och kontrollsekvenser.

### Schematiskt flödesschema
```
 (Clock)───▶┌────────────────┐
           │ Turing Machine ├─▶ (Gate) gate_out
 (Reset)───▶│ (Length,       │
           │  Mutate, Lock) ├─▶ (CV) cv_out
           └────────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `clock` | Gate | En klockpuls som stegar sekvensern. |
| `reset` | Gate | En puls som återställer sekvensen. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `gate_out` | Gate | En utgående gate-signal. |
| `cv_out` | CV | En utgående kontrollspänning (tonhöjd eller annan parameter). |

### Parametrar

*   **Length**: Längden på sekvensen (1-16 steg).
*   **Mutation Rate**: Hur ofta ett slumpmässigt bitbyte sker i den interna sekvensen. Högre värden ger mer kaos.
*   **Lock Length**: Antalet steg från slutet som ska "låsas" och inte muteras. Detta låter dig behålla en del av mönstret stabilt medan resten utvecklas.
*   **Lock Position**: Den specifika position som ska "låsas".
*   **Scale**: Väljer en musikalisk skala som CV-utgången kvantiseras till, så att melodierna alltid är i tonart.
---
## Random Gates
**Kategori:** Sequencer

Random Gates-modulen genererar slumpmässiga gate-signaler. Användaren kan ställa in sannolikheten för att en gate ska triggas vid varje inkommande klockpuls. Detta är perfekt för att skapa intressanta, icke-repetitiva rytmer, fyllningar, eller för att slumpmässigt trigga event i synten.

### Schematiskt flödesschema
```
 (Clock)───▶┌─────────────┐
           │ Random Gates├─▶ (Gate) out
 (Prob)────▶│ (Prob, Seed)│
           └─────────────┘
```

### Ingångar (Inputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `clock` | Gate | En klockpuls som triggar modulen att generera en slumpmässig gate. |
| `probability_cv` | CV | Modulerar sannolikheten för att en gate ska triggas. |

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Gate | Den slumpmässigt genererade gate-signalen. |

### Parametrar

*   **Probability**: Sannolikheten (0.0 till 1.0) för att en gate ska triggas vid varje klockpuls.
*   **Seed**: Ett startvärde för slumpgeneratorn. Samma seed ger samma sekvens av "slumpmässiga" gates, vilket gör det möjligt att återskapa specifika mönster.
*   **Length**: Längden på den utgående gate-pulsen.
---
## BBD Delay (Bucket Brigade Delay)
**Kategori:** Effect

En BBD Delay simulerar ljudet av ett klassiskt analogt "Bucket Brigade Device"-delay. Dessa enheter var kända för sin varma, organiska karaktär och sin gradvisa nedbrytning av ljudkvaliteten vid längre fördröjningar och högre feedback. Denna modul emulerar dessa egenskaper, inklusive bandbreddsbegränsning och distorsion i feedbackloopen, för att ge en äkta retro-fördröjning.

### Schematiskt flödesschema
```
(in)──▶┌─────────┐
      │ BBD Delay ├─▶ (Audio) out
      │ (Time,   │
      │  Feedb.) │
      └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Time**: Fördröjningstiden. Längre tider ger en mörkare och mer distorderad delay.
*   **Feedback**: Hur mycket av den fördröjda signalen som matas tillbaka in i delaylinjen. Höga värden kan leda till självsvängning.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (fördröjd signal).
*   **Tone**: Ett filter i feedbackloopen som simulerar bandbreddsbegränsningen hos en fysisk BBD-enhet.
*   **Drive**: Överstyrning i feedbackloopen för ytterligare distorsion.
---
## Chorus
**Kategori:** Effect

Chorus-effekten simulerar ljudet av flera instrument eller röster som spelar samma ton med små, slumpmässiga variationer i tonhöjd och timing. Denna modul uppnår detta genom att duplicera insignalen, försena kopiorna med en kort, moduleringstid, och sedan blanda dem med originalet. Resultatet är ett tjockare, bredare och "svävande" ljud.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │  Chorus ├─▶ (Audio) out_l
(in_r)──▶│ (Rate,  │
        │  Depth, )├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Rate**: Hastigheten på LFO:n som modulerar delay-tiden.
*   **Depth**: Hur intensiv modulations LFO:n är, vilket påverkar hur mycket tonhöjden "svajar".
*   **Mix**: Balansen mellan dry (originalsignal) och wet (chorus-effekten).
*   **Voices**: Antalet fördröjda kopior som skapas. Fler röster ger en fylligare, mer komplex effekt.
---
## Compressor
**Kategori:** Effect

Kompressorn är en dynamisk effekt som automatiskt reducerar volymskillnaderna i en ljudsignal. Den sänker volymen på höga ljud (över en viss tröskel) och kan förstärka tysta ljud, vilket gör ljudet mer jämnt och "punchigt". Denna modul erbjuder justerbara attack- och release-tider samt möjlighet till sidechain-filtrering för mer kontrollerad ducking.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │ Compres.├─▶ (Audio) out_l
(in_r)──▶│ (Thresh,│
        │ Ratio,  ├─▶ (Audio) out_r
(sc_in)─▶│ Attack) │
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Threshold**: Nivån i decibel då kompressorn börjar arbeta. Ljud över denna nivå kommer att komprimeras.
*   **Ratio**: Hur mycket ljudet över tröskeln komprimeras. T.ex. ett ratio på 4:1 betyder att för varje 4dB som ljudet går över tröskeln, släpps bara 1dB igenom.
*   **Attack**: Tiden det tar för kompressorn att reagera fullt ut på en signal som överskrider tröskeln. Kort attack = snabb kompression, lång attack = mer transienter släpps igenom.
*   **Release**: Tiden det tar för kompressorn att sluta komprimera efter att signalen sjunkit under tröskeln. Kort release = snabb återgång till normal volym, lång release = mjukare återgång.
*   **Makeup**: Ytterligare förstärkning som läggs till efter kompression för att kompensera för den förlorade volymen.
*   **Mix**: Balansen mellan dry (okomprimerad signal) och wet (komprimerad signal). Används för parallell kompression.
*   **Sidechain**: Aktiverar extern sidechain-ingång.
*   **SC Filter**: Högpassfilter för sidechain-signalen, för att undvika att basfrekvenser triggar kompressorn för mycket.
---
## Convolver
**Kategori:** Effect

Convolver-modulen implementerar en "convolution reverb", vilket är en typ av reverb som återskapar efterklangen från ett specifikt fysiskt rum eller akustiskt utrymme. Detta görs genom att använda en "impulsrespons" (IR) – en inspelning av hur rummet reagerar på en kort, kraftig ljudpuls. Modulen använder matematiskt genererade IR:er för plattor, rum, fjädrar och hallar, vilket ger en högkvalitativ och varierande rumssimulering.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │ Convolver├─▶ (Audio) out_l
(in_r)──▶│ (IR Type,│
        │ Decay,  )├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **IR Type**: Väljer vilken typ av impulssvar som ska användas.
    *   `Plate`: Simulerar en plåtreverb, känd för sin ljusa, jämna och täta efterklang.
    *   `Room`: Simulerar en typisk rumsakustik.
    *   `Spring`: Simulerar en fjäderreverb, med en karaktäristisk "twangy" och vibrerande efterklang.
    *   `Hall`: Simulerar efterklangen i en stor konserthall, med en fyllig och lång efterklang.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (reverb-effekten).
*   **Pre-Delay**: Tiden det tar innan reverb-effekten börjar höras efter originalsignalen. Simulerar ljudets gång till rummets första reflektioner.
*   **Decay**: Kontrollerar hur snabbt efterklangen klingar ut.
*   **Brightness**: Justerar klangfärgen på reverbet, ofta genom ett högpass- eller lågpassfilter i feedbackloopen.
---
## Delay
**Kategori:** Effect

Delay-effekten upprepar en inkommande ljudsignal efter en viss tid. Denna modul erbjuder en mångsidig stereo-delay med oberoende fördröjningstider för vänster och höger kanal, tempo-synkronisering till syntens BPM, ping-pong-läge där ekon "studsar" mellan stereokanalerna, samt feedback med mjuk limiter för att förhindra oönskad självsvängning.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │  Delay  ├─▶ (Audio) out_l
(in_r)──▶│ (Time,  │
        │ Feedback)├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Mode**: Väljer delay-läge.
    *   `Mono`: Mono-delay med samma inställningar för båda kanalerna.
    *   `Stereo`: Oberoende delay-tider för vänster och höger kanal.
    *   `Ping-Pong`: Ekon växlar mellan vänster och höger kanal.
*   **Time**: Fördröjningstid i sekunder. När Tempo Sync är på, ställs denna in i förhållande till BPM.
*   **Feedback**: Hur mycket av den fördröjda signalen som matas tillbaka in i delaylinjen, vilket skapar upprepade ekon.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (fördröjd signal).
*   **Tone**: Ett filter i feedbackloopen som dämpar höga frekvenser, vilket gör att ekon blir mörkare och mjukare över tid.
*   **Tempo Sync**: Aktiverar eller inaktiverar synkronisering av delay-tiden till syntens BPM.
*   **Sync Division**: När Tempo Sync är aktiv, väljer denna parameter notvärdet för delay-tiden (t.ex. kvartsnot, åttondelsnot).
---
## Distortion
**Kategori:** Effect

Distortion-effekten tillför "grus", "värme" eller aggressivitet till ett ljud genom att klippa eller forma om vågformen. Denna modul erbjuder flera olika distorsionstyper, från mjuk mättnad till hård klippning och bitcrushing, samt ett "tone"-filter för att forma klangfärgen på det distorderade ljudet.

### Schematiskt flödesschema
```
(in)──▶┌─────────┐
      │ Distort.├─▶ (Audio) out
      │ (Type,  │
      │  Drive) │
      └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den distorderade utgångssignalen. |

### Parametrar

*   **Type**: Väljer distorsionsalgoritm.
    *   `Soft Clip`: Mjuk klippning som simulerar rörförstärkare eller bandmättnad.
    *   `Hard Clip`: Hård klippning vid en definierad nivå, vilket ger en aggressiv, fuzz-liknande distorsion.
    *   `Foldback`: Vik-distorsion, där signalen "fälls tillbaka" när den överskrider ett visst tröskelvärde, vilket skapar komplexa övertoner.
    *   `Bitcrush`: Reducerar bitdjupet, vilket skapar digitala artefakter och ett "lo-fi"-ljud.
    *   `Tube`: Asymmetrisk mjuk klippning som simulerar rörförstärkare.
*   **Drive**: Mängden distorsion. Ökar insignalens volym innan distorsion appliceras.
*   **Tone**: Ett filter som formar klangfärgen efter distorsionen, från mörkt till ljust.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (distorderad signal).
*   **Bit Depth**: Endast relevant för `Bitcrush`-läge. Ställer in bitdjupet (1-16 bitar).
---
## EQ (Equalizer)
**Kategori:** Effect

EQ-modulen är en 3-bands parametrisk equalizer med låg-hylla, mid-peak och hög-hylla filter. Den används för att forma frekvensbalansen i ett ljud genom att förstärka eller dämpa specifika frekvensområden.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │   EQ    ├─▶ (Audio) out_l
(in_r)──▶│ (Low,   │
        │  Mid,   ├─▶ (Audio) out_r
        │  High)  │
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Low Freq**: Frekvensen för låg-hylla filtret. Allt under denna frekvens påverkas.
*   **Low Gain**: Förstärkning eller dämpning (i decibel) av de låga frekvenserna.
*   **Mid Freq**: Centrumfrekvensen för mid-peak filtret.
*   **Mid Gain**: Förstärkning eller dämpning (i decibel) av mid-frekvensområdet.
*   **Mid Q**: "Q-faktor" för mid-peak filtret, vilket bestämmer hur brett eller smalt frekvensområdet är som påverkas. Högre Q = smalare band.
*   **High Freq**: Frekvensen för hög-hylla filtret. Allt över denna frekvens påverkas.
*   **High Gain**: Förstärkning eller dämpning (i decibel) av de höga frekvenserna.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (EQ-behandlad signal).
---
## Flanger
**Kategori:** Effect

Flanger-effekten skapar ett virvlande, "jetplan"-liknande ljud genom att blanda en signal med en kort, tidsvarierande fördröjd kopia av sig själv. Fördröjningstiden moduleras med en LFO. Denna modul erbjuder klassisk flanger med feedback för intensiva, svepande effekter.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │ Flanger ├─▶ (Audio) out_l
(in_r)──▶│ (Rate,  │
        │  Depth) ├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Rate**: Hastigheten på LFO:n som modulerar delay-tiden.
*   **Depth**: Hur mycket LFO:n modulerar delay-tiden.
*   **Feedback**: Mängden fördröjd signal som matas tillbaka in i delaylinjen, vilket förstärker effekten. Kan vara positiv (förstärkande) eller negativ (filtrerande).
*   **Delay**: Grundläggande fördröjningstid i millisekunder.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (flanger-effekten).
---
## Limiter
**Kategori:** Effect

Limitern är en brickwall-kompressor med ett oändligt kompressionsförhållande, vilket innebär att ingen ljudsignal kan överstiga en definierad "ceiling"-nivå. Denna modul har även "look-ahead" (förutseende), vilket gör att den kan reagera på transienter innan de uppstår, vilket förhindrar överstyrning på ett transparent sätt utan hörbara artefakter. Används för att skydda system och maximera den upplevda volymen.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │ Limiter ├─▶ (Audio) out_l
(in_r)──▶│ (Ceiling,│
        │ Release)├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Ceiling**: Den maximala utgångsnivån (i decibel) som ljudet kommer att tillåtas nå.
*   **Look-Ahead**: Tiden (i millisekunder) som limitern "tittar in i framtiden" för att upptäcka och reagera på toppar innan de inträffar.
*   **Release**: Tiden det tar för gain-reduktionen att återgå till noll efter att signalen har sjunkit under taknivån.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (limiterad signal). Normalt 100% wet för en limiter.
---
## Mid/Side
**Kategori:** Effect

Mid/Side-modulen bearbetar stereoljud genom att dela upp det i en "Mid"-komponent (summan av vänster och höger kanal, det vill säga monoljudet) och en "Side"-komponent (skillnaden mellan vänster och höger kanal, det vill säga stereoinformationen). Detta gör det möjligt att manipulera monoljudet och stereoljudet oberoende av varandra, till exempel för att justera stereobredden eller lägga till effekter bara på sidokanalen.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │ Mid/Side├─▶ (Audio) out_l
(in_r)──▶│ (Width, │
        │  Gain)  ├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Width**: Kontrollerar stereobredden.
    *   `0.0`: Kollar ihop signalen till mono.
    *   `0.5`: Normal stereobredd.
    *   `1.0`: Extremt bred stereo.
*   **Mid Gain**: Justerar volymen på Mid-kanalen (monokomponenten).
*   **Side Gain**: Justerar volymen på Side-kanalen (stereokomponenten).
*   **Mix**: Balansen mellan dry (originalsignal) och wet (mid/side-behandlad signal).
---
## Phase Vocoder
**Kategori:** Effect

Phase Vocoder-modulen är en avancerad effekt för "spektral" ljudmanipulation. Den kan utföra realtids pitch-shifting (ändra tonhöjd utan att ändra tempo) och spektral frysning ("freeze"). I "freeze"-läge kan modulen fånga och hålla den aktuella klangfärgen, vilket skapar långa, eteriska ljudtexturer från kortare signaler.

### Schematiskt flödesschema
```
(in_l)──▶┌────────────────┐
        │ Phase Vocoder  ├─▶ (Audio) out_l
(in_r)──▶│ (Pitch Shift,  │
        │  Freeze)       ├─▶ (Audio) out_r
        └────────────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Pitch Shift**: Ändrar tonhöjden i halvtoner (semitoner).
*   **Freeze**: När aktiverad, fryser den nuvarande spektrala informationen, vilket skapar en ihållande ton eller textur.
*   **FFT Size**: Storleken på FFT-beräkningen (Fast Fourier Transform). Större FFT-storlek ger högre frekvensupplösning (mer exakt pitch-shifting) men längre latens.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (vocoder-effekten).
---
## Phaser
**Kategori:** Effect

Phaser-effekten skapar en svepande, "whooshing" eller "swooshing" ljud genom att skapa fasförskjutningar i en signal med hjälp av en serie all-pass-filter. En LFO modulerar centrumfrekvensen för dessa filter, vilket skapar den karaktäristiska svepande effekten. Denna modul simulerar klassiska phaser-kretsar med feedback för att förstärka effekten.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │ Phaser  ├─▶ (Audio) out_l
(in_r)──▶│ (Rate,  │
        │  Depth) ├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Rate**: Hastigheten på LFO:n som modulerar fasfiltren.
*   **Depth**: Hur mycket LFO:n påverkar fasfilterens centrumfrekvens.
*   **Feedback**: Mängden av den fasförskjutna signalen som matas tillbaka in i filtren, vilket förstärker effekten och ger den en mer resonant karaktär.
*   **Center Freq**: Den genomsnittliga centrumfrekvensen för LFO-svepet.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (phaser-effekten).
---
## Reverb
**Kategori:** Effect

Reverb-modulen skapar en efterklang som simulerar ljudet av en signal som spelas upp i ett akustiskt utrymme. Denna modul använder en "Feedback Delay Network" (FDN) med 8 kanaler och en Hadamard-matris för att skapa täta, diffusa reflektioner. Den erbjuder kontroller för rummets storlek, efterklangstid, dämpning av höga frekvenser, samt möjlighet till pre-delay och stereobredd.

### Schematiskt flödesschema
```
(in_l)──▶┌─────────┐
        │ Reverb  ├─▶ (Audio) out_l
(in_r)──▶│ (Room,  │
        │  Decay) ├─▶ (Audio) out_r
        └─────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out_l` | Audio | Vänster stereoutgång. |
| `out_r` | Audio | Höger stereoutgång. |

### Parametrar

*   **Room Size**: Simulerar storleken på det virtuella rummet, vilket påverkar hur lång tid det tar för de första reflektionerna att uppstå.
*   **Decay**: Kontrollerar den totala efterklangstiden.
*   **Damping**: Hur snabbt höga frekvenser dämpas i efterklangen, vilket simulerar luftabsorption eller mjukare ytor i rummet.
*   **Diffusion**: Hur snabbt de första reflektionerna "smälter samman" till en tät efterklang.
*   **Pre-Delay**: En kort fördröjning innan efterklangen börjar, simulerar avståndet till rummets första ytor.
*   **Low Cut**: Ett högpassfilter som tar bort låga frekvenser från efterklangen för att förhindra att den blir grumlig.
*   **Width**: Kontrollerar stereobredden på reverb-effekten.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (reverb-effekten).
---
## Waveshaper
**Kategori:** Effect

Waveshaper-modulen är en kreativ effekt som deformerar ljudvågor på olika sätt, vilket skapar rika övertoner, distorsion och unika klangfärger. Den erbjuder en rad olika kurvor, från mjuk klippning och asymmetrisk distorsion till wavefolding och Chebysheve-polynom för harmonisk kontroll. Den kan användas för att "skulptera" ljud på ett detaljerat och ibland oförutsägbart sätt.

### Schematiskt flödesschema
```
(in)──▶┌────────────┐
      │ Waveshaper ├─▶ (Audio) out
      │ (Curve,    │
      │  Drive,   )│
      └────────────┘
```

### Ingångar (Inputs)
*   *Inga explicita ljud- eller CV-ingångar.* Effect-moduler processar inkommande ljud i kedjan.

### Utgångar (Outputs)

| Port | Typ | Beskrivning |
|---|---|---|
| `out` | Audio | Den vågformade utgångssignalen. |

### Parametrar

*   **Curve**: Väljer vilken vågforms-algoritm som ska användas.
    *   `SoftClip`: Mjuk tanh-mättnad.
    *   `Asymmetric`: Asymmetrisk distorsion, där positiva och negativa delar av vågformen behandlas olika.
    *   `Fold`: Wavefolder, som "viker" tillbaka signalen när den överskrider ett visst tröskelvärde, vilket skapar rika, metalliska övertoner.
    *   `Chebyshev`: Använder Chebysheve-polynom för att selektivt generera jämna eller udda övertoner.
    *   `SineFold`: sin(x) -liknande distorsion för FM-liknande klangfärger.
    *   `Quantize`: Bit-reducering för "lo-fi"-effekter.
*   **Drive**: Ingångsförstärkning före vågformningen, vilket påverkar hur hårt effekten slår till.
*   **Mix**: Balansen mellan dry (originalsignal) och wet (vågformad signal).
*   **Bias**: En DC-offset som läggs till signalen före vågformningen.
*   **Symmetry**: Kontrollerar den asymmetriska distorsionen för vissa kurvor.
