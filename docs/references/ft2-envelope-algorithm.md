# FT2 Envelope-algoritm (ft2-clone referens)

Extraherat från 8bitbubsy's ft2-clone (ft2_replayer.c), den mest exakta öppen-källkods-implementeringen av FT2-beteende.

**Källa:** https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c

---

## Komplett envelope-processering per tick

Funktionen `updateVolPanAutoVib()` anropas **en gång per tick** för varje kanal.

### Steg 1: Inkrementera envelope-position

```c
ch->volEnvTick++;
```

Position inkrementeras FÖRST, sedan evalueras.

### Steg 2: Kontrollera om vi nått en punkt

```c
if (ch->volEnvTick == ins->volEnvPoints[envPos][0])
{
    ch->fVolEnvValue = (float)(int32_t)(ins->volEnvPoints[envPos][1] & 0xFF);
    envPos++;
}
```

### Steg 3: Sustain-kontroll (bara om noten hålls)

```c
if ((ins->volEnvFlags & ENV_SUSTAIN) && !ch->keyOff)
{
    if (envPos-1 == ins->volEnvSustain)
    {
        envPos--;
        ch->fVolEnvDelta = 0.0f;  // Stoppa interpolering
        envInterpolateFlag = false;
    }
}
```

**Notera:** Sustain kontrolleras EFTER punkt-kontroll. Om vi precis nådde sustain-punkten, stoppas vidare avancering.

### Steg 4: Loop-kontroll

```c
if (ins->volEnvFlags & ENV_LOOP)
{
    envPos--;
    if (envPos == ins->volEnvLoopEnd)
    {
        // Specialfall: om sustain = loop end OCH noten släppts, loopa INTE
        if (!(ins->volEnvFlags & ENV_SUSTAIN) ||
            envPos != ins->volEnvSustain || !ch->keyOff)
        {
            envPos = ins->volEnvLoopStart;
            ch->volEnvTick = ins->volEnvPoints[envPos][0];
            ch->fVolEnvValue = (float)(int32_t)(ins->volEnvPoints[envPos][1] & 0xFF);
        }
    }
    envPos++;
}
```

**Kritiskt:** Sustain har prioritet. Om `sustain == loopEnd` och noten har släppts (`keyOff`), hoppar vi INTE tillbaka till loop start.

### Steg 5: Interpolering

```c
if (envInterpolateFlag)
{
    ch->volEnvPos = envPos;

    // Beräkna delta för linjär interpolering
    if (envPos < ins->volEnvLength)
    {
        int32_t envVal = (int32_t)(ins->volEnvPoints[envPos][1] & 0xFF);
        int32_t envFrameDist = ins->volEnvPoints[envPos][0] - ins->volEnvPoints[envPos-1][0];
        if (envFrameDist > 0)
        {
            float fEnvVal = (float)(envVal - (int32_t)ch->fVolEnvValue);
            ch->fVolEnvDelta = fEnvVal / (float)envFrameDist;
        }
    }
}

ch->fVolEnvValue += ch->fVolEnvDelta;
```

### Steg 6: Fadeout (PARALLELLT med envelope)

```c
if (ch->keyOff)
{
    ch->fadeoutVol -= ch->fadeoutSpeed;  // LINJÄR subtraktion
    if (ch->fadeoutVol <= 0)
    {
        ch->fadeoutVol = 0;
        // Tyst - voice kan återanvändas
    }
}
```

### Steg 7: Slutlig volymberäkning

```c
const int32_t vol = song.globalVolume * ch->outVol * ch->fadeoutVol;
float fVol = vol * (1.0f / (64.0f * 64.0f * 32768.0f));
fVol *= ch->fVolEnvValue * (1.0f / 64.0f);
```

Fullständig formel:
```
FinalVol = (globalVol/64) * (outVol/64) * (fadeoutVol/32768) * (envVol/64)
```

---

## Nyckelinsikter

### 1. Fadeout är LINJÄRT
- `fadeoutVol` startar vid 32768
- Subtraheras med `fadeoutSpeed` (0..4095) varje tick
- INTE multiplikativt/exponentiellt
- Tid till tystnad = 32768 / fadeoutSpeed ticks

### 2. Fadeout och envelope körs PARALLELLT
- Efter key-off: envelope fortsätter från nuvarande position (förbi sustain)
- SAMTIDIGT: fadeout subtraheras varje tick
- Slutgiltig volym = envelope * fadeout * andra faktorer

### 3. Envelope-position inkrementeras FÖRE evaluering
- `volEnvTick++` körs först
- Sedan kontrolleras om vi nått en punkt
- Off-by-one jämfört med "inkrementera efter"

### 4. Sustain prioritet över loop
- Om sustain == loopEnd och noten har släppts → loop hoppas inte
- Envelopet fortsätter förbi loop end

### 5. Envelope-värde 0..64
- Normaliserat: `envVol / 64.0`
- 0 = tystnad, 64 = full volym

### 6. Initial fadeoutVol
- 32768 (inte 65536 som vissa specifikationer anger)
- Normaliserat genom division med 32768
